use crate::error::{AppError, Result};
use reqwest::Client;
use std::net::IpAddr;
use std::time::Duration;
use tracing::{debug, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedIp {
    pub ip: IpAddr,
    pub provider: &'static str,
}

struct Provider {
    name: &'static str,
    url: &'static str,
    is_trace: bool,
}

const PROVIDERS: &[Provider] = &[
    Provider {
        name: "Cloudflare Primary (1.1.1.1)",
        url: "https://1.1.1.1/cdn-cgi/trace",
        is_trace: true,
    },
    Provider {
        name: "Cloudflare Secondary (1.0.0.1)",
        url: "https://1.0.0.1/cdn-cgi/trace",
        is_trace: true,
    },
    Provider {
        name: "ipify",
        url: "https://api.ipify.org",
        is_trace: false,
    },
    Provider {
        name: "ifconfig.me",
        url: "https://ifconfig.me/ip",
        is_trace: false,
    },
    Provider {
        name: "icanhazip",
        url: "https://icanhazip.com",
        is_trace: false,
    },
];

pub struct IpDetector {
    client: Client,
}

impl IpDetector {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(4))
            .user_agent("noeddns/0.1.0")
            .build()?;
        Ok(Self { client })
    }

    pub async fn detect(&self) -> Result<DetectedIp> {
        let mut last_error = None;

        for provider in PROVIDERS {
            debug!("Attempting IP detection via {}", provider.name);
            match self.fetch_from_provider(provider).await {
                Ok(ip) => {
                    if is_valid_public_ip(&ip) {
                        return Ok(DetectedIp {
                            ip,
                            provider: provider.name,
                        });
                    } else {
                        warn!(
                            "Provider {} returned non-public/invalid IP: {}",
                            provider.name, ip
                        );
                    }
                }
                Err(err) => {
                    debug!("Provider {} failed: {}", provider.name, err);
                    last_error = Some(err);
                }
            }
        }

        Err(AppError::IpDetection(
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "All IP detection providers failed".to_string()),
        ))
    }

    async fn fetch_from_provider(&self, provider: &Provider) -> Result<IpAddr> {
        let resp = self.client.get(provider.url).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::IpDetection(format!(
                "HTTP status: {}",
                resp.status()
            )));
        }

        let body = resp.text().await?;
        let ip_str = if provider.is_trace {
            extract_trace_ip(&body).ok_or_else(|| {
                AppError::IpDetection(format!("No ip= line found in trace output from {}", provider.name))
            })?
        } else {
            body.trim()
        };

        let ip: IpAddr = ip_str.parse().map_err(|e| {
            AppError::IpDetection(format!("Failed to parse IP '{}': {}", ip_str, e))
        })?;

        Ok(ip)
    }
}

fn extract_trace_ip(trace_text: &str) -> Option<&str> {
    for line in trace_text.lines() {
        if let Some(stripped) = line.strip_prefix("ip=") {
            let ip = stripped.trim();
            if !ip.is_empty() {
                return Some(ip);
            }
        }
    }
    None
}

fn is_valid_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_loopback()
                && !v4.is_unspecified()
                && !v4.is_link_local()
                && !v4.is_broadcast()
                && !v4.is_documentation()
                && !v4.is_private()
        }
        IpAddr::V6(v6) => {
            !v6.is_loopback() && !v6.is_unspecified() && !v6.is_multicast()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_trace_ip() {
        let trace = "fl=123f\nh=1.1.1.1\nip=203.0.113.195\nts=1600000000\nvisit_scheme=https\nuag=noeddns\n";
        assert_eq!(extract_trace_ip(trace), Some("203.0.113.195"));
    }

    #[test]
    fn test_invalid_ips() {
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(!is_valid_public_ip(&loopback));

        let lan: IpAddr = "192.168.1.1".parse().unwrap();
        assert!(!is_valid_public_ip(&lan));

        let public_v4: IpAddr = "1.1.1.1".parse().unwrap();
        assert!(is_valid_public_ip(&public_v4));
    }
}
