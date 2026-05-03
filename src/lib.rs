//! Tibber CLI library — fetches and caches energy price data from the Tibber API.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, ValueEnum};
use directories::ProjectDirs;

/// Tibber GraphQL API client — home discovery and energy price retrieval.
pub mod api;

/// Controls when price data is fetched from the API.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum FetchMode {
    /// Only fetch when cached data is missing or stale (after 14:00 without
    /// next-day prices).
    #[default]
    Smart,
    /// Always fetch fresh data from the API.
    Force,
}

/// Controls the output format for price display.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum PriceFormat {
    /// Full output: `home: price currency`.
    #[default]
    Full,
    /// Short output: price amount only (e.g. `0.2977`).
    Short,
}

/// Command-line arguments for the Tibber CLI.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    /// The Tibber API access token
    #[arg(short, long, env = "TIBBER_ACCESS_TOKEN", hide_env_values = true)]
    pub access_token: String,

    /// Home ID to operate on (auto-selected when there is only one home)
    #[arg(long, env = "TIBBER_HOME_ID")]
    pub home_id: Option<api::HomeId>,

    /// When to fetch fresh price data from the API
    #[arg(short, long, value_enum, default_value_t = FetchMode::Smart)]
    pub fetch_mode: FetchMode,

    /// Show the current energy price
    #[arg(short, long)]
    pub show_current_price: bool,

    /// Output format for price display
    #[arg(short, long, value_enum, default_value_t = PriceFormat::Full)]
    pub output_format: PriceFormat,

    /// Fetch and cache energy prices, then exit.
    ///
    /// Intended for periodic use (e.g. cron every hour). Respects --fetch-mode.
    #[arg(short = 'p', long)]
    pub fetch_prices: bool,

    /// Never make API calls — only read from cache.
    ///
    /// Useful for fast, offline price lookups (e.g. cron every 15 min with
    /// --show-current-price). Fails if the cache is empty.
    #[arg(long)]
    pub cache_only: bool,

    /// List all available homes and exit
    #[arg(short, long)]
    pub list_homes: bool,

    /// Clear all cached data (homes and prices) and exit
    #[arg(short, long)]
    pub clear_cache: bool,
}

/// Runs the CLI application logic.
///
/// # Errors
///
/// This function will return an error if the underlying operation fails.
pub fn run(cli: &Cli) -> anyhow::Result<()> {
    let cache_dir = get_cache_dir()?;

    if cli.clear_cache {
        clear_cache(&cache_dir)?;
        return Ok(());
    }

    let homes = load_or_fetch_homes(&cli.access_token, &cache_dir, cli.cache_only)?;

    if cli.list_homes {
        print_homes(&homes);
        return Ok(());
    }

    let home = resolve_home(cli.home_id.as_ref(), &homes)?;

    if cli.fetch_prices {
        fetch_and_cache_prices(&cli.access_token, home, cli.fetch_mode, &cache_dir)?;
        return Ok(());
    }

    if cli.show_current_price {
        show_current_price_cmd(
            &cli.access_token,
            home,
            cli.cache_only,
            cli.output_format,
            &cache_dir,
        )?;
    }

    Ok(())
}

/// Resolves the current energy price and prints it.
///
/// Reads from cache first. If the cache is missing or doesn't contain the
/// current time slot, falls back to an API fetch (unless `cache_only` is set).
///
/// # Errors
///
/// Returns an error if the cache cannot be read or the API request fails.
fn show_current_price_cmd(
    access_token: &str,
    home: &api::Home,
    cache_only: bool,
    format: PriceFormat,
    cache_dir: &Path,
) -> anyhow::Result<()> {
    let cache_path = cache_dir.join(format!("prices-{}.json", home.id));
    let mut price_data = load_price_cache(&cache_path)?;

    let has_price = price_data
        .as_ref()
        .is_some_and(|d| d.current_price().is_some());

    if !has_price && !cache_only {
        price_data = Some(fetch_and_write_prices(access_token, home, cache_dir)?);
    }

    if let Some(data) = &price_data {
        print_current_price(home, data, format);
    } else {
        anyhow::bail!("{}: no current price available", home.name);
    }

    Ok(())
}

/// Removes the entire cache directory and prints a confirmation.
///
/// Does nothing if the directory does not exist.
///
/// # Errors
///
/// Returns an error if the directory exists but cannot be removed.
fn clear_cache(cache_dir: &Path) -> anyhow::Result<()> {
    if cache_dir.exists() {
        std::fs::remove_dir_all(cache_dir).context("failed to clear cache directory")?;
    }
    println!("Cache cleared.");
    Ok(())
}

/// Loads homes from the cache file, or fetches them from the API and caches the
/// result.
///
/// When `cache_only` is `true`, only the cache is consulted and a missing cache
/// is treated as an error.
///
/// # Errors
///
/// Returns an error if the cache cannot be read/written, if the API request
/// fails, or if `cache_only` is set and the cache is missing.
fn load_or_fetch_homes(
    access_token: &str,
    cache_dir: &Path,
    cache_only: bool,
) -> anyhow::Result<Vec<api::Home>> {
    let cache_path = cache_dir.join("homes.json");

    if cache_path.exists() {
        let content =
            std::fs::read_to_string(&cache_path).context("failed to read homes cache file")?;
        return serde_json::from_str(&content).context("failed to parse homes cache file");
    }

    if cache_only {
        anyhow::bail!(
            "no cached homes found; run without --cache-only first to populate the cache"
        );
    }

    println!("Fetching homes from Tibber API...");
    let homes = api::fetch_homes(access_token)?;

    let content =
        serde_json::to_string_pretty(&homes).context("failed to serialize homes for cache")?;
    std::fs::write(&cache_path, content).context("failed to write homes cache file")?;

    Ok(homes)
}

/// Selects which home to operate on.
///
/// If `home_id` is provided, the matching home is returned. If there is exactly
/// one home, it is selected automatically. Otherwise an error is returned
/// requesting the user to disambiguate.
///
/// # Errors
///
/// Returns an error if the requested home ID is not found, or if multiple homes
/// exist without a selection.
fn resolve_home<'a>(
    home_id: Option<&api::HomeId>,
    homes: &'a [api::Home],
) -> anyhow::Result<&'a api::Home> {
    match (home_id, homes.len()) {
        (Some(id), _) => homes
            .iter()
            .find(|h| h.id == *id)
            .with_context(|| format!("home with ID '{id}' not found")),
        (None, 1) => Ok(&homes[0]),
        (None, _) => {
            anyhow::bail!(
                "multiple homes found; use --home-id to select one, \
                 or --list-homes to see available homes"
            );
        }
    }
}

/// Fetches energy prices from the API and writes them to the cache, respecting
/// the [`FetchMode`]:
/// - **Force** always fetches.
/// - **Smart** fetches only when the cache is missing or stale.
///
/// # Errors
///
/// Returns an error if the API request fails or the cache cannot be written.
fn fetch_and_cache_prices(
    access_token: &str,
    home: &api::Home,
    fetch_mode: FetchMode,
    cache_dir: &Path,
) -> anyhow::Result<()> {
    let cache_path = cache_dir.join(format!("prices-{}.json", home.id));

    let needs_fetch = match fetch_mode {
        FetchMode::Force => true,
        FetchMode::Smart => {
            let cached = load_price_cache(&cache_path)?;
            cached.is_none_or(|d| d.should_fetch_new_price_data(home.time_zone))
        }
    };

    if needs_fetch {
        fetch_and_write_prices(access_token, home, cache_dir)?;
        println!("Prices cached for {}.", home.name);
    } else {
        println!("Cache is up-to-date for {}.", home.name);
    }

    Ok(())
}

/// Fetches energy prices from the API and writes them to the cache file.
///
/// # Errors
///
/// Returns an error if the API request fails or the cache cannot be written.
fn fetch_and_write_prices(
    access_token: &str,
    home: &api::Home,
    cache_dir: &Path,
) -> anyhow::Result<api::PriceData> {
    let cache_path = cache_dir.join(format!("prices-{}.json", home.id));

    println!("Fetching energy prices for {}...", home.name);
    let data = api::fetch_prices(access_token, &home.id)?;

    let content =
        serde_json::to_string_pretty(&data).context("failed to serialize price data for cache")?;
    std::fs::write(&cache_path, &content).context("failed to write price cache file")?;

    Ok(data)
}

/// Reads and deserializes the price cache file, returning `None` if it does not
/// exist.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or parsed.
fn load_price_cache(cache_path: &Path) -> anyhow::Result<Option<api::PriceData>> {
    if !cache_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(cache_path).context("failed to read price cache file")?;
    let data: api::PriceData =
        serde_json::from_str(&content).context("failed to parse price cache file")?;
    Ok(Some(data))
}

/// Prints the list of available homes to stdout.
fn print_homes(homes: &[api::Home]) {
    println!("Found {} home(s):", homes.len());
    for home in homes {
        println!("  {} (ID: {}, TZ: {})", home.name, home.id, home.time_zone);
    }
}

/// Prints the current energy price for a home to stdout.
fn print_current_price(home: &api::Home, price_data: &api::PriceData, format: PriceFormat) {
    if let Some(price) = price_data.current_price() {
        match format {
            PriceFormat::Full => {
                let currency = price_data.currency.as_deref().unwrap_or("?");
                println!("{}: {price} {currency}", home.name);
            }
            PriceFormat::Short => println!("{price}"),
        }
    } else {
        println!("{}: no current price available", home.name);
    }
}

/// Determines the cache location, creates it if it doesn't exist, and returns
/// it.
///
/// # Errors
///
/// Returns an error if the project directory cannot be determined, or if
/// creating the directory on the filesystem fails.
fn get_cache_dir() -> anyhow::Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("nl", "erwin314", "tibber")
        .context("could not determine project directories")?;
    let cache_dir = proj_dirs.cache_dir();
    std::fs::create_dir_all(cache_dir).context("failed to create cache directory")?;
    Ok(cache_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a [`api::Home`] with the given ID and name, using UTC as timezone.
    fn home(id: &str, name: &str) -> api::Home {
        api::Home {
            id: api::HomeId(id.to_owned()),
            name: name.to_owned(),
            time_zone: chrono_tz::UTC,
        }
    }

    // -- resolve_home ----------------------------------------------------------

    #[test]
    fn resolve_home_auto_selects_single_home() {
        // Arrange
        let homes = [home("abc", "My House")];

        // Act
        let result = resolve_home(None, &homes);

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, api::HomeId("abc".to_owned()));
    }

    #[test]
    fn resolve_home_finds_matching_id() {
        // Arrange
        let homes = [home("aaa", "Home A"), home("bbb", "Home B")];
        let target = api::HomeId("bbb".to_owned());

        // Act
        let result = resolve_home(Some(&target), &homes);

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap().name, "Home B");
    }

    #[test]
    fn resolve_home_errors_on_unknown_id() {
        // Arrange
        let homes = [home("aaa", "Home A")];
        let target = api::HomeId("zzz".to_owned());

        // Act
        let result = resolve_home(Some(&target), &homes);

        // Assert
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("zzz"), "error should mention the unknown ID");
    }

    #[test]
    fn resolve_home_errors_when_multiple_homes_without_selection() {
        // Arrange
        let homes = [home("aaa", "Home A"), home("bbb", "Home B")];

        // Act
        let result = resolve_home(None, &homes);

        // Assert
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("--home-id"),
            "error should hint at --home-id flag"
        );
    }
}
