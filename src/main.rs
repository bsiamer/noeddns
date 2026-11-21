mod cloudflare;
mod config;
mod db;
mod error;
mod ip;
mod systemd;

use clap::Parser;
use cloudflare::{CloudflareClient, SyncOutcome};
use config::{AppConfig, Cli, Commands, ServiceAction};
use db::Database;
use error::Result;
use ip::IpDetector;
use std::process;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();

    let res = match &cli.command {
        Some(Commands::Service { action }) => handle_service_command(action),
        Some(Commands::Status) => handle_status_command(&cli).await,
        Some(Commands::Sync) | None => handle_sync_command(&cli).await,
    };

    if let Err(err) = res {
        error!("Error: {}", err);
        process::exit(1);
    }
}

fn handle_service_command(action: &ServiceAction) -> Result<()> {
    let mgr = systemd::SystemdManager::new()?;
    match action {
        ServiceAction::Install => mgr.install(),
        ServiceAction::Uninstall => mgr.uninstall(),
        ServiceAction::Status => mgr.status(),
        ServiceAction::Logs { lines } => mgr.logs(*lines),
    }
}

async fn handle_status_command(cli: &Cli) -> Result<()> {
    println!("=== noeddns Status ===");

    // Check current public IP
    let detector = IpDetector::new()?;
    match detector.detect().await {
        Ok(detected) => {
            println!("Current Public IP: {} (via {})", detected.ip, detected.provider);
        }
        Err(e) => {
            println!("Current Public IP: Unable to detect ({})", e);
        }
    }

    // Check DB
    let db_path = cli
        .db
        .clone()
        .unwrap_or_else(config::default_db_path);

    if db_path.exists() {
        let db = Database::open(&db_path)?;
        let cached = db.get_last_cached_ip()?;
        println!("Last Cached IP   : {}", cached.as_deref().unwrap_or("None"));
        println!("Database Path    : {}", db_path.display());

        let events = db.get_recent_events(10)?;
        if !events.is_empty() {
            println!("\nRecent Sync Events (up to 10):");
            println!(
                "{:<20} {:<12} {:<24} {:<16} {:<10} {:<8}",
                "Time", "Zone", "Record", "IP", "Status", "Proxied"
            );
            println!("{}", "-".repeat(95));
            for ev in events {
                println!(
                    "{:<20} {:<12} {:<24} {:<16} {:<10} {:<8}",
                    ev.synced_at,
                    ev.zone,
                    ev.record_name,
                    ev.ip_address,
                    ev.status,
                    ev.proxied
                );
            }
        }
    } else {
        println!("Database Path    : {} (not initialized yet)", db_path.display());
    }

    Ok(())
}

async fn handle_sync_command(cli: &Cli) -> Result<()> {
    let config = AppConfig::load(cli)?;
    let db = Database::open(&config.db_path)?;

    info!("Detecting public IP address...");
    let detector = IpDetector::new()?;
    let detected = detector.detect().await?;
    let ip_str = detected.ip.to_string();
    let version_str = match detected.ip {
        std::net::IpAddr::V4(_) => "IPv4",
        std::net::IpAddr::V6(_) => "IPv6",
    };

    info!(
        "Detected public {} address: {} (provider: {})",
        version_str, ip_str, detected.provider
    );

    let last_cached_ip = db.get_last_cached_ip()?;
    let ip_changed = last_cached_ip.as_deref() != Some(&ip_str);

    if !ip_changed && !config.force {
        info!(
            "Public IP unchanged ({}) since last check. Skipping Cloudflare updates.",
            ip_str
        );
        return Ok(());
    }

    if config.force {
        info!("Force flag specified. Proceeding with Cloudflare sync.");
    } else {
        info!(
            "IP changed: '{}' -> '{}'. Updating Cloudflare DNS records...",
            last_cached_ip.as_deref().unwrap_or("none"),
            ip_str
        );
    }

    // Save detected IP to DB
    db.save_detected_ip(&ip_str, version_str, detected.provider)?;

    let cf = CloudflareClient::new(config.api_token)?;
    let zone_id = cf.get_zone_id(&config.zone).await?;
    info!("Resolved zone '{}' -> Zone ID: {}", config.zone, zone_id);

    let rec_type = match detected.ip {
        std::net::IpAddr::V4(_) => "A",
        std::net::IpAddr::V6(_) => "AAAA",
    };

    let mut any_error = false;

    for record_name in &config.records {
        match cf
            .sync_record(&zone_id, record_name, &detected.ip, config.proxied, config.ttl)
            .await
        {
            Ok(outcome) => {
                let status_str = match outcome {
                    SyncOutcome::Unchanged => "UNCHANGED",
                    SyncOutcome::Updated { .. } => "UPDATED",
                    SyncOutcome::Created { .. } => "CREATED",
                };

                db.save_sync_event(
                    &config.zone,
                    record_name,
                    rec_type,
                    &ip_str,
                    config.proxied,
                    status_str,
                    None,
                )?;

                info!("Record {}: {}", record_name, status_str);
            }
            Err(e) => {
                any_error = true;
                let err_msg = e.to_string();
                warn!("Failed to sync record {}: {}", record_name, err_msg);

                let _ = db.save_sync_event(
                    &config.zone,
                    record_name,
                    rec_type,
                    &ip_str,
                    config.proxied,
                    "ERROR",
                    Some(&err_msg),
                );
            }
        }
    }

    if any_error {
        Err(error::AppError::Cloudflare(
            "One or more records failed to sync with Cloudflare".to_string(),
        ))
    } else {
        info!("All DNS records successfully synchronized.");
        Ok(())
    }
}
