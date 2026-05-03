//! Tibber API types and GraphQL client implementation.

use std::collections::BTreeMap;

use anyhow::{Context, bail};
use chrono::{DateTime, FixedOffset, Timelike};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// The URL for the Tibber GraphQL API.
pub const TIBBER_GRAPHQL_ENDPOINT: &str = "https://api.tibber.com/v1-beta/gql";

/// Unique identifier for a Tibber home.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct HomeId(pub String);

impl std::fmt::Display for HomeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for HomeId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_owned()))
    }
}

/// A Tibber home with its resolved display name and timezone.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Home {
    /// Unique identifier for the home.
    pub id: HomeId,
    /// Display name (from app nickname, address, or fallback).
    pub name: String,
    /// IANA timezone of the home (e.g. `Europe/Amsterdam`).
    pub time_zone: Tz,
}

#[derive(Deserialize)]
struct GraphQlResponse<T> {
    data: Data<T>,
}

#[derive(Deserialize)]
struct Data<T> {
    viewer: T,
}

/// Fetches the list of homes from the Tibber API.
///
/// The display name is resolved from `appNickname`, falling back to `address1`,
/// and finally to `"Tibber Home"`.
///
/// # Errors
///
/// Returns an error if the network request fails, if the API response cannot be
/// parsed, or if no homes are found.
pub fn fetch_homes(access_token: &str) -> anyhow::Result<Vec<Home>> {
    #[derive(Deserialize)]
    struct Viewer {
        homes: Vec<RawHome>,
    }
    #[derive(Deserialize)]
    struct RawHome {
        id: HomeId,
        #[serde(rename = "timeZone")]
        time_zone: Tz,
        #[serde(rename = "appNickname")]
        app_nickname: Option<String>,
        address: Option<Address>,
    }
    #[derive(Deserialize)]
    struct Address {
        address1: Option<String>,
    }

    let query = r"
        {
          viewer {
            homes {
              id
              timeZone
              appNickname
              address {
                address1
              }
            }
          }
        }
    ";

    let mut response = ureq::post(TIBBER_GRAPHQL_ENDPOINT)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({ "query": query }))
        .context("failed to send request to Tibber API")?;

    let parsed: GraphQlResponse<Viewer> = response
        .body_mut()
        .read_json()
        .context("failed to parse Tibber API response")?;

    let raw_homes = parsed.data.viewer.homes;
    if raw_homes.is_empty() {
        bail!("no homes found");
    }

    let homes = raw_homes
        .into_iter()
        .map(|h| {
            let name = h
                .app_nickname
                .filter(|n| !n.is_empty())
                .or_else(|| h.address.and_then(|a| a.address1).filter(|a| !a.is_empty()))
                .unwrap_or_else(|| "Tibber Home".to_owned());
            Home {
                id: h.id,
                name,
                time_zone: h.time_zone,
            }
        })
        .collect();

    Ok(homes)
}

/// Price data returned by [`fetch_prices`].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PriceData {
    /// Mapping of `startsAt` timestamp → price total.
    pub prices: BTreeMap<DateTime<FixedOffset>, f64>,
    /// Currency code (e.g. `"SEK"`, `"NOK"`), if available.
    pub currency: Option<String>,
    /// The last `startsAt` timestamp from the API's first-day ("today") array.
    /// After this point, the fetched data is considered stale.
    pub stale_after: DateTime<FixedOffset>,
    /// Whether the API response included a non-empty next-day price list.
    pub includes_next_day: bool,
}

impl PriceData {
    /// Returns `true` if the price data contains valid next-day entries.
    ///
    /// Requires both that the API returned next-day prices and that the local
    /// clock has not yet passed [`stale_after`](Self::stale_after).
    #[must_use]
    #[inline]
    pub fn has_tomorrow_prices(&self) -> bool {
        self.has_tomorrow_prices_at(&chrono::Utc::now())
    }

    #[must_use]
    #[inline]
    fn has_tomorrow_prices_at<Tz: chrono::TimeZone>(&self, now: &DateTime<Tz>) -> bool {
        self.includes_next_day
            && now.with_timezone(&chrono::Utc) <= self.stale_after.with_timezone(&chrono::Utc)
    }

    /// Returns `true` if new price data should be fetched from the API.
    ///
    /// This is the case when tomorrow's prices are missing and the current time
    /// in the home's timezone is past 14:00 (the typical window when next-day
    /// prices become available).
    #[must_use]
    #[inline]
    pub fn should_fetch_new_price_data(&self, home_tz: Tz) -> bool {
        self.should_fetch_new_price_data_at(home_tz, &chrono::Utc::now())
    }

    #[must_use]
    #[inline]
    fn should_fetch_new_price_data_at<Tz: chrono::TimeZone>(
        &self,
        home_tz: chrono_tz::Tz,
        now: &DateTime<Tz>,
    ) -> bool {
        !self.has_tomorrow_prices_at(now) && now.with_timezone(&home_tz).hour() >= 14
    }

    /// Returns the price for the current time slot, if available.
    ///
    /// Finds the last entry whose `startsAt` is at or before the current time,
    /// which corresponds to the active 15-minute price slot.
    #[must_use]
    #[inline]
    pub fn current_price(&self) -> Option<f64> {
        self.current_price_at(chrono::Local::now().fixed_offset())
    }

    #[must_use]
    #[inline]
    fn current_price_at(&self, now: DateTime<FixedOffset>) -> Option<f64> {
        self.prices
            .range(..=now)
            .next_back()
            .map(|(_, &total)| total)
    }
}

/// Fetches energy prices for a single home.
///
/// Uses a GraphQL variable to filter by `home_id`. Today and tomorrow price
/// points are merged into a single flat map keyed by their `startsAt` timestamp.
///
/// # Errors
///
/// Returns an error if the network request fails, if the API response cannot be
/// parsed, if the home or subscription is not found, or if no price data is
/// returned.
pub fn fetch_prices(access_token: &str, home_id: &HomeId) -> anyhow::Result<PriceData> {
    #[derive(Deserialize)]
    struct Viewer {
        home: Option<HomeNode>,
    }
    #[derive(Deserialize)]
    struct HomeNode {
        #[serde(rename = "currentSubscription")]
        current_subscription: Option<Subscription>,
    }
    #[derive(Deserialize)]
    struct Subscription {
        #[serde(rename = "priceInfo")]
        price_info: Option<PriceInfo>,
    }
    #[derive(Deserialize)]
    struct PriceInfo {
        current: Option<CurrentPrice>,
        #[serde(default)]
        today: Vec<PricePoint>,
        #[serde(default)]
        tomorrow: Vec<PricePoint>,
    }
    #[derive(Deserialize)]
    struct CurrentPrice {
        currency: Option<String>,
    }
    #[derive(Deserialize)]
    struct PricePoint {
        total: f64,
        #[serde(rename = "startsAt")]
        starts_at: DateTime<FixedOffset>,
    }

    let query = r"
        query($homeId: ID!) {
          viewer {
            home(id: $homeId) {
              currentSubscription {
                priceInfo(resolution: QUARTER_HOURLY) {
                  current {
                    currency
                  }
                  today {
                    total
                    startsAt
                  }
                  tomorrow {
                    total
                    startsAt
                  }
                }
              }
            }
          }
        }
    ";

    let mut response = ureq::post(TIBBER_GRAPHQL_ENDPOINT)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({
            "query": query,
            "variables": { "homeId": &home_id.0 },
        }))
        .context("failed to send request for prices to Tibber API")?;

    let parsed: GraphQlResponse<Viewer> = response
        .body_mut()
        .read_json()
        .context("failed to parse Tibber API prices response")?;

    let home = parsed.data.viewer.home.context("home not found")?;

    let price_info = home
        .current_subscription
        .and_then(|s| s.price_info)
        .context("no price info found (active subscription?)")?;

    let currency = price_info.current.and_then(|c| c.currency);

    let stale_after = price_info
        .today
        .iter()
        .map(|p| p.starts_at)
        .max()
        .context("no today prices returned from API")?;

    let includes_next_day = !price_info.tomorrow.is_empty();

    let prices: BTreeMap<DateTime<FixedOffset>, f64> = price_info
        .today
        .into_iter()
        .chain(price_info.tomorrow)
        .map(|p| (p.starts_at, p.total))
        .collect();

    Ok(PriceData {
        prices,
        currency,
        stale_after,
        includes_next_day,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use chrono_tz::Europe::Oslo;

    #[test]
    fn test_has_tomorrow_prices() {
        let stale_after = Utc
            .with_ymd_and_hms(2023, 1, 1, 23, 0, 0)
            .unwrap()
            .fixed_offset();

        let mut data = PriceData {
            prices: BTreeMap::new(),
            currency: None,
            stale_after,
            includes_next_day: true,
        };

        // Before stale_after -> valid
        let now = Utc.with_ymd_and_hms(2023, 1, 1, 12, 0, 0).unwrap();
        assert!(data.has_tomorrow_prices_at(&now));

        // After stale_after -> invalid
        let now = Utc.with_ymd_and_hms(2023, 1, 2, 0, 0, 1).unwrap();
        assert!(!data.has_tomorrow_prices_at(&now));

        // includes_next_day = false -> invalid
        data.includes_next_day = false;
        let now = Utc.with_ymd_and_hms(2023, 1, 1, 12, 0, 0).unwrap();
        assert!(!data.has_tomorrow_prices_at(&now));
    }

    #[test]
    fn test_should_fetch_new_price_data() {
        let stale_after = Utc
            .with_ymd_and_hms(2023, 1, 1, 23, 0, 0)
            .unwrap()
            .fixed_offset();

        let mut data = PriceData {
            prices: BTreeMap::new(),
            currency: None,
            stale_after,
            includes_next_day: false,
        };

        // 13:00 Oslo time -> shouldn't fetch
        let now = Oslo.with_ymd_and_hms(2023, 1, 1, 13, 0, 0).unwrap();
        assert!(!data.should_fetch_new_price_data_at(Oslo, &now));

        // 14:00 Oslo time -> should fetch
        let now = Oslo.with_ymd_and_hms(2023, 1, 1, 14, 0, 0).unwrap();
        assert!(data.should_fetch_new_price_data_at(Oslo, &now));

        // If we already have next day -> shouldn't fetch
        data.includes_next_day = true;
        assert!(!data.should_fetch_new_price_data_at(Oslo, &now));
    }

    #[test]
    fn test_current_price() {
        let mut prices = BTreeMap::new();
        let t1 = Utc
            .with_ymd_and_hms(2023, 1, 1, 10, 0, 0)
            .unwrap()
            .fixed_offset();
        let t2 = Utc
            .with_ymd_and_hms(2023, 1, 1, 11, 0, 0)
            .unwrap()
            .fixed_offset();

        prices.insert(t1, 1.5);
        prices.insert(t2, 2.5);

        let data = PriceData {
            prices,
            currency: None,
            stale_after: t2,
            includes_next_day: false,
        };

        // Before first -> None
        let now = Utc
            .with_ymd_and_hms(2023, 1, 1, 9, 0, 0)
            .unwrap()
            .fixed_offset();
        assert_eq!(data.current_price_at(now), None);

        // At first -> 1.5
        let now = Utc
            .with_ymd_and_hms(2023, 1, 1, 10, 0, 0)
            .unwrap()
            .fixed_offset();
        assert_eq!(data.current_price_at(now), Some(1.5));

        // Between first and second -> 1.5
        let now = Utc
            .with_ymd_and_hms(2023, 1, 1, 10, 30, 0)
            .unwrap()
            .fixed_offset();
        assert_eq!(data.current_price_at(now), Some(1.5));

        // At second -> 2.5
        let now = Utc
            .with_ymd_and_hms(2023, 1, 1, 11, 0, 0)
            .unwrap()
            .fixed_offset();
        assert_eq!(data.current_price_at(now), Some(2.5));

        // After second -> 2.5 (until next fetch)
        let now = Utc
            .with_ymd_and_hms(2023, 1, 1, 12, 0, 0)
            .unwrap()
            .fixed_offset();
        assert_eq!(data.current_price_at(now), Some(2.5));
    }
}
