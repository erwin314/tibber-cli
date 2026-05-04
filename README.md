# tibber-cli

A fast, cache-aware CLI for querying energy prices from the [Tibber](https://tibber.com) API.

Designed for automation: fetch prices on a schedule, then look them up instantly from cache — no API call needed.

## Features

- **Smart caching** — only fetches new data when tomorrow's prices become available (after 14:00 local time)
- **Offline lookups** — `--cache-only` reads from disk in <1 ms, ideal for frequent cron jobs
- **Multi-home support** — auto-selects when you have one home, prompts when you have several
- **Machine-friendly output** — `--output-format short` prints just the price number

## Installation

```sh
cargo install --path .
```

Or build a release binary:

```sh
cargo build --release
# Binary at target/release/tibber-cli
```

## Configuration

Create a `.env` file or export the environment variable:

```sh
# .env
TIBBER_ACCESS_TOKEN=your-token-here
```

Get your access token at [developer.tibber.com](https://developer.tibber.com).

Optionally pin a specific home:

```sh
TIBBER_HOME_ID=xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
```

## Usage

### List your homes

```sh
tibber-cli list-homes
```

### Fetch and cache prices

```sh
# Smart mode (default): only fetches when cache is stale
tibber-cli fetch-prices

# Force a fresh fetch
tibber-cli --fetch-mode force fetch-prices
```

### Show current price

```sh
# Full output: "My Home: 0.2977 SEK"
tibber-cli show-current-price

# Machine-friendly: "0.2977"
tibber-cli show-current-price --output-format short
```

### Offline price lookup

```sh
# Never hits the API — reads from cache only
tibber-cli --cache-only show-current-price
```

### Clear cache

```sh
tibber-cli clear-cache
```

## Cron Examples

A two-job setup for reliable, low-latency price lookups:

```crontab
# Fetch prices every hour (smart mode skips when cache is fresh)
0 * * * * tibber-cli fetch-prices

# Log current price every 15 minutes (instant, offline)
*/15 * * * * tibber-cli --cache-only show-current-price --output-format short >> /var/log/energy-price.log
```

## Development

```sh
# Run tests
cargo test

# Lint (pedantic, deny level)
cargo clippy

# Format
cargo fmt
```

## License

[Apache-2.0](LICENSE)
