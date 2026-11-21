use crate::error::{AppError, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "noeddns",
    author = "Boussad Siamer <boussadsiamer8@gmail.com>",
    version = "0.1.0",
    about = "Zero-idle-memory Cloudflare Dynamic DNS client for Linux"
)]
pub struct Cli {
    #[arg(short, long, global = true, value_name = "FILE", help = "Path to config file")]
    pub config: Option<PathBuf>,

    #[arg(short, long, global = true, env = "CLOUDFLARE_API_TOKEN", help = "Cloudflare API Bearer token")]
    pub token: Option<String>,

    #[arg(short, long, global = true, env = "NOEDDNS_ZONE", help = "Cloudflare Zone name (e.g. siamer.xyz)")]
    pub zone: Option<String>,

    #[arg(
        short,
        long,
        global = true,
        env = "NOEDDNS_RECORDS",
        value_delimiter = ',',
        help = "Comma-separated DNS records to update (e.g. siamer.xyz,www.siamer.xyz)"
    )]
    pub records: Option<Vec<String>>,

    #[arg(short, long, global = true, help = "Enable Cloudflare CDN proxy (orange cloud)")]
    pub proxied: Option<bool>,

    #[arg(long, global = true, help = "DNS record TTL in seconds (1 = automatic)")]
    pub ttl: Option<u32>,

    #[arg(long, global = true, help = "Path to SQLite database file")]
    pub db: Option<PathBuf>,

    #[arg(long, global = true, help = "Force Cloudflare update even if cached IP matches")]
    pub force: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(about = "Run a single sync check and update Cloudflare if IP changed (default)")]
    Sync,

    #[command(about = "Display current IP, cached IP, and recent sync history")]
    Status,

    #[command(about = "Manage the Linux native systemd timer (0 idle memory)")]
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    #[command(about = "Install and enable systemd user timer (wakes up every 1m, 0MB RAM when idle)")]
    Install,

    #[command(about = "Uninstall and remove systemd user timer & service")]
    Uninstall,

    #[command(about = "Show systemd timer status")]
    Status,

    #[command(about = "Show recent journal logs for noeddns.service")]
    Logs {
        #[arg(short, long, default_value_t = 30, help = "Number of log lines to show")]
        lines: usize,
    },
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct FileConfig {
    pub api_token: Option<String>,
    pub zone: Option<String>,
    pub records: Option<Vec<String>>,
    pub proxied: Option<bool>,
    pub ttl: Option<u32>,
    pub db_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub api_token: String,
    pub zone: String,
    pub records: Vec<String>,
    pub proxied: bool,
    pub ttl: u32,
    pub db_path: PathBuf,
    pub force: bool,
}

impl AppConfig {
    pub fn load(cli: &Cli) -> Result<Self> {
        let file_cfg = Self::find_and_load_config(cli.config.as_deref())?;

        let api_token = cli
            .token
            .clone()
            .or(file_cfg.api_token)
            .ok_or_else(|| {
                AppError::Config(
                    "Cloudflare API token not provided. Set CLOUDFLARE_API_TOKEN, pass --token, or configure api_token in config.toml".to_string(),
                )
            })?;

        let zone = cli
            .zone
            .clone()
            .or(file_cfg.zone)
            .ok_or_else(|| {
                AppError::Config(
                    "Zone name not provided. Set NOEDDNS_ZONE, pass --zone, or configure zone in config.toml".to_string(),
                )
            })?;

        let records = cli
            .records
            .clone()
            .or(file_cfg.records)
            .unwrap_or_else(|| vec![zone.clone()]);

        let proxied = cli.proxied.or(file_cfg.proxied).unwrap_or(false);
        let ttl = cli.ttl.or(file_cfg.ttl).unwrap_or(1);

        let default_db = default_db_path();
        let db_path = cli.db.clone().or(file_cfg.db_path).unwrap_or(default_db);

        Ok(Self {
            api_token,
            zone,
            records,
            proxied,
            ttl,
            db_path,
            force: cli.force,
        })
    }

    fn find_and_load_config(explicit_path: Option<&Path>) -> Result<FileConfig> {
        let search_paths: Vec<PathBuf> = if let Some(p) = explicit_path {
            vec![p.to_path_buf()]
        } else {
            let mut paths = vec![PathBuf::from("config.toml")];
            if let Ok(home) = std::env::var("HOME") {
                paths.push(Path::new(&home).join(".config/noeddns/config.toml"));
            }
            paths.push(PathBuf::from("/etc/noeddns/config.toml"));
            paths
        };

        for path in search_paths {
            if path.exists() {
                let content = fs::read_to_string(&path).map_err(|e| {
                    AppError::Config(format!("Failed to read {}: {}", path.display(), e))
                })?;
                let cfg: FileConfig = toml::from_str(&content).map_err(|e| {
                    AppError::Config(format!("Failed to parse TOML in {}: {}", path.display(), e))
                })?;
                return Ok(cfg);
            }
        }

        Ok(FileConfig::default())
    }
}

pub fn default_db_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        Path::new(&home).join(".local/share/noeddns/ddns.sqlite")
    } else {
        PathBuf::from("ddns.sqlite")
    }
}
