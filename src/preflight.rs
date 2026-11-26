//! System checks: `rust-openconnect-gui --check` runs these headless, and the
//! Preflight button in the GUI runs the same list.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::{config, secrets, vpn};

#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

pub fn run_checks() -> Vec<Check> {
    vec![
        check_binaries(),
        check_sudoers(),
        check_keyring(),
        check_profile(),
        check_config_dir(),
    ]
}

/// Is the NOPASSWD sudoers rule for openconnect in place? Decides whether
/// the Setup card needs to be shown at all.
pub fn sudoers_ok() -> bool {
    Command::new(vpn::SUDO)
        .args(["-n", vpn::OPENCONNECT, "--version"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn check_binaries() -> Check {
    let paths = [vpn::OPENCONNECT, vpn::SUDO, vpn::PGREP, vpn::PKILL];
    let missing: Vec<&str> = paths.iter().copied().filter(|p| !Path::new(p).exists()).collect();
    if missing.is_empty() {
        Check { name: "required tools", ok: true, detail: "openconnect, sudo, pgrep, pkill found".into() }
    } else {
        Check { name: "required tools", ok: false, detail: format!("missing: {}", missing.join(", ")) }
    }
}

fn check_sudoers() -> Check {
    let mut ok = true;
    let mut detail = String::new();

    // `sudo -n openconnect --version` exercises the NOPASSWD rule harmlessly.
    match Command::new(vpn::SUDO).args(["-n", vpn::OPENCONNECT, "--version"]).output() {
        Ok(o) if o.status.success() => detail.push_str("openconnect can run without a password"),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            ok = false;
            if err.contains("password is required") {
                detail.push_str("sudoers rule not set up — see Setup in the app");
            } else {
                detail.push_str(&format!("sudo -n failed: {}", err.trim()));
            }
        }
        Err(e) => {
            ok = false;
            detail.push_str(&format!("could not run sudo: {e}"));
        }
    }

    // Probing pkill would kill a live VPN, so only check it when nothing runs.
    // pkill exits 1 when it matched nothing; sudo's auth failure is what we look for.
    if vpn::is_running() {
        detail.push_str("; pkill rule skipped (VPN is running)");
    } else {
        match Command::new(vpn::SUDO)
            .args(["-n", vpn::PKILL, "-INT", "-x", "openconnect"])
            .output()
        {
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                if err.contains("password is required") {
                    ok = false;
                    detail.push_str("; pkill rule missing — add it to the sudoers file");
                } else {
                    detail.push_str("; pkill rule OK");
                }
            }
            Err(e) => {
                ok = false;
                detail.push_str(&format!("; could not probe pkill: {e}"));
            }
        }
    }

    Check { name: "sudoers rules", ok, detail }
}

fn check_keyring() -> Check {
    match secrets::probe() {
        Ok(detail) => Check { name: "password keyring", ok: true, detail },
        Err(e) => Check { name: "password keyring", ok: false, detail: e },
    }
}

fn check_profile() -> Check {
    let cfg = config::load();
    if !cfg.server.trim().is_empty() && !cfg.username.trim().is_empty() {
        Check { name: "profile configured", ok: true, detail: format!("{} as {}", cfg.server, cfg.username) }
    } else {
        Check { name: "profile configured", ok: false, detail: "server and/or username not set in the app yet".into() }
    }
}

fn check_config_dir() -> Check {
    let Some(dir) = config::config_path().map(|p| p.parent().map(|d| d.to_path_buf())).flatten() else {
        return Check { name: "config directory", ok: false, detail: "could not determine".into() };
    };
    match std::fs::create_dir_all(&dir) {
        Ok(()) => Check { name: "config directory", ok: true, detail: dir.display().to_string() },
        Err(e) => Check { name: "config directory", ok: false, detail: format!("{e}") },
    }
}
