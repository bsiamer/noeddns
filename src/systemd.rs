use crate::error::{AppError, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

pub struct SystemdManager {
    unit_dir: PathBuf,
    binary_path: PathBuf,
}

impl SystemdManager {
    pub fn new() -> Result<Self> {
        let home = std::env::var("HOME")
            .map_err(|_| AppError::Other("Could not determine HOME directory".to_string()))?;
        let unit_dir = Path::new(&home).join(".config/systemd/user");

        let binary_path = std::env::current_exe()
            .map_err(|e| AppError::Other(format!("Failed to determine binary path: {}", e)))?;

        Ok(Self {
            unit_dir,
            binary_path,
        })
    }

    pub fn install(&self) -> Result<()> {
        fs::create_dir_all(&self.unit_dir)?;

        let service_path = self.unit_dir.join("noeddns.service");
        let timer_path = self.unit_dir.join("noeddns.timer");

        let exe_str = self.binary_path.to_string_lossy();

        let service_content = format!(
            r#"[Unit]
Description=noeddns Cloudflare Dynamic DNS Updater
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart={} sync
StandardOutput=journal
StandardError=journal
"#,
            exe_str
        );

        let timer_content = r#"[Unit]
Description=Run noeddns every minute
Requires=noeddns.service

[Timer]
OnCalendar=*:0/1
Persistent=true
AccuracySec=1s

[Install]
WantedBy=timers.target
"#;

        fs::write(&service_path, service_content)?;
        info!("Wrote service unit to {}", service_path.display());

        fs::write(&timer_path, timer_content)?;
        info!("Wrote timer unit to {}", timer_path.display());

        // Reload systemd daemon
        self.run_systemctl(&["daemon-reload"])?;

        // Enable and start the timer immediately
        self.run_systemctl(&["enable", "--now", "noeddns.timer"])?;

        info!("Successfully installed and started noeddns.timer! (Runs every minute with 0 memory when idle)");
        Ok(())
    }

    pub fn uninstall(&self) -> Result<()> {
        let timer_path = self.unit_dir.join("noeddns.timer");
        let service_path = self.unit_dir.join("noeddns.service");

        let _ = self.run_systemctl(&["disable", "--now", "noeddns.timer"]);

        if timer_path.exists() {
            fs::remove_file(&timer_path)?;
            info!("Removed {}", timer_path.display());
        }

        if service_path.exists() {
            fs::remove_file(&service_path)?;
            info!("Removed {}", service_path.display());
        }

        let _ = self.run_systemctl(&["daemon-reload"]);
        info!("Successfully uninstalled noeddns systemd service and timer.");
        Ok(())
    }

    pub fn status(&self) -> Result<()> {
        let output = Command::new("systemctl")
            .args(["--user", "status", "noeddns.timer"])
            .output()
            .map_err(|e| AppError::Other(format!("Failed to run systemctl: {}", e)))?;

        println!("{}", String::from_utf8_lossy(&output.stdout));
        if !output.stderr.is_empty() {
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }

        // Also check timer next elapse
        let list_output = Command::new("systemctl")
            .args(["--user", "list-timers", "noeddns.timer"])
            .output();
        if let Ok(lo) = list_output {
            println!("{}", String::from_utf8_lossy(&lo.stdout));
        }

        Ok(())
    }

    pub fn logs(&self, lines: usize) -> Result<()> {
        let count_arg = format!("-n{}", lines);
        let output = Command::new("journalctl")
            .args(["--user", "-u", "noeddns.service", &count_arg, "--no-pager"])
            .output()
            .map_err(|e| AppError::Other(format!("Failed to run journalctl: {}", e)))?;

        println!("{}", String::from_utf8_lossy(&output.stdout));
        if !output.stderr.is_empty() {
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }

    fn run_systemctl(&self, args: &[&str]) -> Result<()> {
        let mut cmd = Command::new("systemctl");
        cmd.arg("--user");
        cmd.args(args);

        let output = cmd
            .output()
            .map_err(|e| AppError::Other(format!("systemctl error: {}", e)))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Other(format!(
                "systemctl {:?} failed: {}",
                args, err
            )));
        }

        Ok(())
    }
}
