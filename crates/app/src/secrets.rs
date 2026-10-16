//! API keys and tokens live in the OS keyring (Secret Service), never in the
//! database. `ANTHROPIC_API_KEY` in the environment also works for Claude.

pub const ANTHROPIC: &str = "anthropic_api_key";
pub const GITHUB: &str = "github_token";
pub const LINEAR: &str = "linear_api_key";
pub const NOTION: &str = "notion_token";
pub const GOOGLE_SECRET: &str = "google_client_secret";
pub const GOOGLE_REFRESH: &str = "google_refresh_token";

fn entry(name: &str) -> Option<keyring::Entry> {
    keyring::Entry::new("flowday", name).ok()
}

pub fn get(name: &str) -> Option<String> {
    let stored = entry(name).and_then(|e| e.get_password().ok()).filter(|s| !s.is_empty());
    match name {
        ANTHROPIC => stored.or_else(|| std::env::var("ANTHROPIC_API_KEY").ok().filter(|s| !s.is_empty())),
        _ => stored,
    }
}

/// Saves a secret; an empty value removes it.
pub fn set(name: &str, value: &str) -> Result<(), String> {
    let e = entry(name).ok_or("can't reach the keyring")?;
    if value.is_empty() {
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    } else {
        e.set_password(value).map_err(|err| format!("can't save to the keyring: {err}"))
    }
}
