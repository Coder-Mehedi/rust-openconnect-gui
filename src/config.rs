use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const APP_NAME: &str = "rust-openconnect-gui";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: String,
    pub username: String,
    pub save_password: bool,
    pub auto_connect: bool,
    /// Pinned server certificate fingerprint, passed to openconnect as
    /// `--servercert=...` so connections don't need interactive confirmation.
    #[serde(default)]
    pub servercert: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: String::new(),
            username: String::new(),
            save_password: false,
            auto_connect: false,
            servercert: String::new(),
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join("config.toml"))
}

/// Directory for runtime state (the openconnect log), e.g. `~/.local/state`.
pub fn app_state_dir() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join(APP_NAME))
}

pub fn log_path() -> Option<PathBuf> {
    app_state_dir().map(|d| d.join("openconnect.log"))
}

pub fn load() -> Config {
    match config_path().and_then(|p| fs::read_to_string(p).ok()) {
        Some(text) => toml::from_str(&text).unwrap_or_default(),
        None => Config::default(),
    }
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let path = config_path().ok_or("could not determine config directory")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("could not create config dir: {e}"))?;
    }
    let text = toml::to_string_pretty(cfg).map_err(|e| format!("could not serialize config: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("could not write config: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let cfg = Config {
            server: "198.51.100.1".into(),
            username: "alice".into(),
            save_password: true,
            auto_connect: true,
            servercert: "9f86d081884c7d65".into(),
        };
        let text = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.server, cfg.server);
        assert_eq!(back.servercert, cfg.servercert);
        assert!(back.save_password);
    }

    #[test]
    fn config_missing_fields_defaults() {
        let parsed: Config = toml::from_str("server = \"x\"\nusername = \"u\"\nsave_password = false\nauto_connect = false\n").unwrap();
        assert_eq!(parsed.servercert, "");
    }
}
