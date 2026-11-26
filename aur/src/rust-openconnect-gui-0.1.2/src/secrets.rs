//! Password storage via the freedesktop Secret Service (gnome-keyring).

use keyring::v1::{Entry, Error};

const SERVICE: &str = "rust-openconnect-gui";
const USER: &str = "vpn-password";

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, USER).map_err(|e| format!("keyring unavailable: {e}"))
}

/// Load the saved VPN password, if any.
pub fn load() -> Result<Option<String>, String> {
    let entry = entry()?;
    match entry.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("could not read keyring: {e}")),
    }
}

pub fn store(password: &str) -> Result<(), String> {
    entry()?.set_password(password).map_err(|e| format!("could not save password to keyring: {e}"))
}

pub fn forget() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) => Ok(()),
        // Nothing saved yet — that's the same end state the user wants.
        Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("could not delete password from keyring: {e}")),
    }
}

/// Used by preflight: proves the Secret Service is reachable without changing anything.
pub fn probe() -> Result<String, String> {
    let entry = entry()?;
    match entry.get_password() {
        Ok(_) => Ok("reachable — saved password present".into()),
        Err(Error::NoEntry) => Ok("reachable — no password saved yet".into()),
        Err(e) => Err(format!("not reachable: {e}")),
    }
}
