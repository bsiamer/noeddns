# noeddns

> **The ultimate Cloudflare Dynamic DNS (DDNS) client for Linux with zero idle memory footprint.**

`noeddns` is a blazingly fast, resilient Cloudflare DDNS client written in Rust. It was designed from the ground up for Linux to monitor and update Cloudflare DNS records (`A` and `AAAA`) with **0.0 MB RAM usage when idle**.

---

## ✨ Key Features

- **0 Idle Memory Usage**: Runs via native Linux `systemd.timer` every minute. Between checks, the process terminates completely—consuming **0 bytes of RAM** and **0% CPU**.
- **1-Command Service Management**: Run `noeddns service install` to automatically generate, install, and start the systemd user timer.
- **Smart Update Caching**: Public IP is checked against an embedded SQLite state cache. If your IP has not changed, Cloudflare API calls are skipped to save rate limits and quota.
- **Multi-Source Resilient IP Detection**: Probes Cloudflare trace (`1.1.1.1` and `1.0.0.1` direct IP), `ipify.org`, `ifconfig.me`, and `icanhazip.com`. Validates IP syntax and rejects private/loopback addresses.
- **Crash-Proof SQLite Event Log**: Configured with Write-Ahead Logging (`PRAGMA journal_mode = WAL`) to store IP change history and Cloudflare sync events safely across power cuts or reboots.
- **Resilient Cloudflare Client**: Automatically handles Cloudflare rate limits (HTTP 429) and transient server errors (500/502/503/504) with backoff and retries.
- **Dual-Stack Support**: Automatically detects IPv4 (`A` records) and IPv6 (`AAAA` records).
- **Flexible Configuration**: Supports `config.toml`, environment variables (`CLOUDFLARE_API_TOKEN`, `NOEDDNS_ZONE`), and CLI flags.

---

## 🚀 Quick Start

### 1. Build & Install Binary

```bash
git clone https://github.com/bsiamer/noeddns.git
cd noeddns
cargo build --release
cp target/release/noeddns ~/.local/bin/   # or /usr/local/bin/
```

### 2. Configure

Create `~/.config/noeddns/config.toml` (or `./config.toml`):

```toml
# Cloudflare API Token (requires Zone.DNS:Edit permissions)
api_token = "your_cloudflare_api_token"

# Your domain zone
zone = "siamer.xyz"

# Records to update
records = [
    "siamer.xyz",
    "www.siamer.xyz"
]

# Cloudflare proxy (orange cloud)
proxied = false

# TTL (1 = automatic)
ttl = 1
```

*(Alternatively, configure via environment variables: `export CLOUDFLARE_API_TOKEN="your_token"` and `export NOEDDNS_ZONE="siamer.xyz"`)*

### 3. Run a Manual Sync

```bash
noeddns sync
```

### 4. Enable 1-Minute Zero-Memory Timer

Install and start the systemd user timer with one command:

```bash
noeddns service install
```

Check the timer schedule and status:

```bash
noeddns service status
```

Inspect recent sync logs:

```bash
noeddns service logs
```

---

## 🛠️ CLI Commands & Flags

```text
noeddns [COMMAND] [OPTIONS]
```

### Commands:
- `sync` *(default)*: Detects public IP, checks local cache, and syncs Cloudflare records if changed.
- `status`: Displays current public IP, cached IP from database, and recent sync events.
- `service install`: Installs and starts the systemd user timer (`noeddns.timer`).
- `service uninstall`: Disables and removes the systemd timer and service files.
- `service status`: Shows systemd timer status and next execution time.
- `service logs`: Views recent systemd journal logs.

### Flags:
| Flag | Environment Variable | Description |
| :--- | :--- | :--- |
| `-c, --config <FILE>` | | Path to TOML config file |
| `-t, --token <TOKEN>` | `CLOUDFLARE_API_TOKEN` | Cloudflare API Bearer token |
| `-z, --zone <ZONE>` | `NOEDDNS_ZONE` | Cloudflare Zone name (e.g. `siamer.xyz`) |
| `-r, --records <NAMES>` | `NOEDDNS_RECORDS` | Comma-separated record names |
| `-p, --proxied` | | Enable Cloudflare CDN proxy (orange cloud) |
| `--ttl <SECONDS>` | | DNS record TTL (1 = automatic) |
| `--db <PATH>` | | Path to SQLite database file |
| `--force` | | Force Cloudflare update even if cached IP matches |

---

## 📊 Database & Auditing

`noeddns` maintains an audit trail in SQLite (`~/.local/share/noeddns/ddns.sqlite`):

- **`ip_history`**: Every public IP transition, timestamp, and the detection provider that verified it.
- **`dns_sync_events`**: Record of every Cloudflare API sync (`CREATED`, `UPDATED`, `UNCHANGED`, `ERROR`) and response details.

View recent sync events anytime with:
```bash
noeddns status
```

---

## 📄 License

BSD 2-Clause License. See [LICENSE](LICENSE) for details.
