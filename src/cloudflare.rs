use crate::error::{AppError, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::Duration;
use tracing::{debug, info, warn};

const CF_API_BASE: &str = "https://api.cloudflare.com/client/v4";

#[derive(Debug, Deserialize)]
struct CfResponse<T> {
    success: bool,
    #[serde(default)]
    errors: Vec<CfError>,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct CfError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct ZoneItem {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DnsRecord {
    pub id: String,
    #[allow(dead_code)]
    pub name: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub record_type: String,
    pub content: String,
    pub proxied: bool,
    #[allow(dead_code)]
    pub ttl: u32,
}

#[derive(Debug, Serialize)]
struct RecordPayload<'a> {
    #[serde(rename = "type")]
    record_type: &'a str,
    name: &'a str,
    content: &'a str,
    ttl: u32,
    proxied: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    Unchanged,
    Updated { old_ip: String, new_ip: String },
    Created { ip: String },
}

pub struct CloudflareClient {
    client: Client,
}

impl CloudflareClient {
    pub fn new(api_token: String) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let auth_val = format!("Bearer {}", api_token);
        let mut auth_header = HeaderValue::from_str(&auth_val)
            .map_err(|e| AppError::Config(format!("Invalid characters in API token: {}", e)))?;
        auth_header.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth_header);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .default_headers(headers)
            .user_agent("noeddns/0.1.0")
            .build()?;

        Ok(Self { client })
    }

    #[allow(dead_code)]
    pub async fn verify_token(&self) -> Result<()> {
        let url = format!("{}/user/tokens/verify", CF_API_BASE);
        let resp: CfResponse<serde_json::Value> = self.get_with_retry(&url).await?;
        if resp.success {
            info!("Cloudflare API token verified successfully.");
            Ok(())
        } else {
            let msg = resp
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.code, e.message))
                .collect::<Vec<_>>()
                .join(", ");
            Err(AppError::Cloudflare(format!("Token verification failed: {}", msg)))
        }
    }

    pub async fn get_zone_id(&self, zone_name: &str) -> Result<String> {
        let url = format!("{}/zones?name={}&status=active", CF_API_BASE, zone_name);
        let resp: CfResponse<Vec<ZoneItem>> = self.get_with_retry(&url).await?;

        if !resp.success {
            let msg = format_errors(&resp.errors);
            return Err(AppError::Cloudflare(format!("Failed to query zones: {}", msg)));
        }

        let zones = resp.result.unwrap_or_default();
        if let Some(zone) = zones.into_iter().find(|z| z.name.eq_ignore_ascii_case(zone_name)) {
            Ok(zone.id)
        } else {
            Err(AppError::Cloudflare(format!(
                "Active zone '{}' not found in Cloudflare account",
                zone_name
            )))
        }
    }

    pub async fn find_dns_record(
        &self,
        zone_id: &str,
        record_name: &str,
        record_type: &str,
    ) -> Result<Option<DnsRecord>> {
        let url = format!(
            "{}/zones/{}/dns_records?name={}&type={}",
            CF_API_BASE, zone_id, record_name, record_type
        );
        let resp: CfResponse<Vec<DnsRecord>> = self.get_with_retry(&url).await?;

        if !resp.success {
            let msg = format_errors(&resp.errors);
            return Err(AppError::Cloudflare(format!("Failed to query DNS records: {}", msg)));
        }

        let records = resp.result.unwrap_or_default();
        Ok(records.into_iter().next())
    }

    pub async fn sync_record(
        &self,
        zone_id: &str,
        record_name: &str,
        ip: &IpAddr,
        proxied: bool,
        ttl: u32,
    ) -> Result<SyncOutcome> {
        let record_type = match ip {
            IpAddr::V4(_) => "A",
            IpAddr::V6(_) => "AAAA",
        };
        let ip_str = ip.to_string();

        let existing = self.find_dns_record(zone_id, record_name, record_type).await?;

        if let Some(record) = existing {
            // Check if update is necessary
            if record.content.trim() == ip_str.trim() && record.proxied == proxied {
                debug!(
                    "DNS record {} ({}) is up to date ({})",
                    record_name, record_type, ip_str
                );
                return Ok(SyncOutcome::Unchanged);
            }

            info!(
                "Updating DNS record {} ({}): {} -> {} (proxied: {})",
                record_name, record_type, record.content, ip_str, proxied
            );

            let url = format!("{}/zones/{}/dns_records/{}", CF_API_BASE, zone_id, record.id);
            let payload = RecordPayload {
                record_type,
                name: record_name,
                content: &ip_str,
                ttl,
                proxied,
            };

            let resp: CfResponse<DnsRecord> = self.put_with_retry(&url, &payload).await?;
            if !resp.success {
                let msg = format_errors(&resp.errors);
                return Err(AppError::Cloudflare(format!("Failed to update record: {}", msg)));
            }

            Ok(SyncOutcome::Updated {
                old_ip: record.content,
                new_ip: ip_str,
            })
        } else {
            info!(
                "DNS record {} ({}) does not exist. Creating with IP {} (proxied: {})",
                record_name, record_type, ip_str, proxied
            );

            let url = format!("{}/zones/{}/dns_records", CF_API_BASE, zone_id);
            let payload = RecordPayload {
                record_type,
                name: record_name,
                content: &ip_str,
                ttl,
                proxied,
            };

            let resp: CfResponse<DnsRecord> = self.post_with_retry(&url, &payload).await?;
            if !resp.success {
                let msg = format_errors(&resp.errors);
                return Err(AppError::Cloudflare(format!("Failed to create record: {}", msg)));
            }

            Ok(SyncOutcome::Created { ip: ip_str })
        }
    }

    async fn get_with_retry<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let resp = self.client.get(url).send().await?;
            let status = resp.status();

            if (status.as_u16() == 429 || status.is_server_error()) && attempts < 3 {
                warn!("Cloudflare request returned {}, retrying in {}s...", status, attempts);
                tokio::time::sleep(Duration::from_secs(attempts)).await;
                continue;
            }

            let parsed = resp.json::<T>().await?;
            return Ok(parsed);
        }
    }

    async fn post_with_retry<T: for<'de> Deserialize<'de>, B: Serialize>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let resp = self.client.post(url).json(body).send().await?;
            let status = resp.status();

            if (status.as_u16() == 429 || status.is_server_error()) && attempts < 3 {
                warn!("Cloudflare POST returned {}, retrying in {}s...", status, attempts);
                tokio::time::sleep(Duration::from_secs(attempts)).await;
                continue;
            }

            let parsed = resp.json::<T>().await?;
            return Ok(parsed);
        }
    }

    async fn put_with_retry<T: for<'de> Deserialize<'de>, B: Serialize>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let resp = self.client.put(url).json(body).send().await?;
            let status = resp.status();

            if (status.as_u16() == 429 || status.is_server_error()) && attempts < 3 {
                warn!("Cloudflare PUT returned {}, retrying in {}s...", status, attempts);
                tokio::time::sleep(Duration::from_secs(attempts)).await;
                continue;
            }

            let parsed = resp.json::<T>().await?;
            return Ok(parsed);
        }
    }
}

fn format_errors(errors: &[CfError]) -> String {
    if errors.is_empty() {
        "Unknown error".to_string()
    } else {
        errors
            .iter()
            .map(|e| format!("{}: {}", e.code, e.message))
            .collect::<Vec<_>>()
            .join("; ")
    }
}
