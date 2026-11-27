//! Application configuration.
//!
//! The configuration comes from environment variables. All values have safe defaults.

use std::collections::HashMap;
use std::path::PathBuf;

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 8765;
pub const LOOPBACK_HOSTS: &[&str] = &["127.0.0.1", "::1", "localhost"];

/// Runtime settings. Use [`Settings::from_env`] to make an instance.
#[derive(Debug, Clone)]
pub struct Settings {
    pub data_dir: PathBuf,
    pub host: String,
    pub port: u16,
    /// Set to true when a TLS reverse proxy is in front of the application.
    pub secure_cookies: bool,
    /// Host header values that the application accepts (DNS rebinding protection).
    pub allowed_hosts: Vec<String>,
    /// Reverse proxies that can give the client address in `X-Forwarded-For`.
    pub trusted_proxies: Vec<crate::net::IpNet>,
    pub session_lifetime_seconds: i64,
    pub max_upload_bytes: usize,
    pub budget_currency: String,
}

/// An absolute path from an environment variable. An empty or relative value is
/// ignored (as the XDG Base Directory specification tells).
fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|path| path.is_absolute())
}

/// The home directory of the user: `HOME`, or `USERPROFILE` on Windows.
fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) { env_dir("USERPROFILE").or_else(|| env_dir("HOME")) } else { env_dir("HOME") }
}

/// The data directory of the operating system, without the name of the application.
fn system_data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env_dir("APPDATA")
    } else if cfg!(target_os = "macos") {
        home_dir().map(|home| home.join("Library").join("Application Support"))
    } else {
        env_dir("XDG_DATA_HOME").or_else(|| home_dir().map(|home| home.join(".local").join("share")))
    }
}

/// The default data directory of the operating system:
///
/// - Linux: `$XDG_DATA_HOME/feljosit` or `~/.local/share/feljosit`
/// - macOS: `~/Library/Application Support/feljosit`
/// - Windows: `%APPDATA%\feljosit`
pub fn default_data_dir() -> PathBuf {
    system_data_dir()
        .or_else(|| home_dir().map(|home| home.join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("feljosit")
}

/// The data directory of version 0.3.0 (the Python version) on all systems:
/// `~/.local/share/feljosit`.
fn legacy_data_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".local").join("share").join("feljosit"))
}

/// The default data directory. If it has no database but the directory of version
/// 0.3.0 has one, use that directory. Thus an update needs no copy (Windows, macOS).
fn data_dir_with_legacy(default: PathBuf, legacy: Option<PathBuf>) -> PathBuf {
    let database = |dir: &PathBuf| dir.join("feljosit.sqlite3").exists();
    match legacy {
        Some(legacy) if !database(&default) && database(&legacy) => legacy,
        _ => default,
    }
}

fn parse_bool(value: Option<&String>, default: bool) -> bool {
    match value {
        None => default,
        Some(v) => matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on"),
    }
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    } else if path == "~"
        && let Some(home) = home_dir()
    {
        return home;
    }
    PathBuf::from(path)
}

impl Settings {
    /// Settings with the default values and the data directory `data_dir`.
    pub fn new(data_dir: PathBuf) -> Settings {
        Settings {
            data_dir,
            host: DEFAULT_HOST.into(),
            port: DEFAULT_PORT,
            secure_cookies: false,
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into(), "::1".into()],
            trusted_proxies: Vec::new(),
            session_lifetime_seconds: 12 * 60 * 60,
            max_upload_bytes: 10 * 1024 * 1024,
            budget_currency: "CHF".into(),
        }
    }

    /// Read the settings from the environment variables with the prefix `FELJOSIT_`.
    pub fn from_env() -> Result<Settings, String> {
        Settings::from_map(&std::env::vars().collect())
    }

    pub fn from_map(env: &HashMap<String, String>) -> Result<Settings, String> {
        let data_dir = env
            .get("FELJOSIT_DATA_DIR")
            .filter(|v| !v.is_empty())
            .map_or_else(|| data_dir_with_legacy(default_data_dir(), legacy_data_dir()), |v| expand_home(v));
        let mut settings = Settings::new(data_dir);
        if let Some(host) = env.get("FELJOSIT_HOST") {
            settings.host = host.clone();
        }
        if let Some(port) = env.get("FELJOSIT_PORT") {
            settings.port =
                port.trim().parse().map_err(|_| "FELJOSIT_PORT must be a port number.".to_string())?;
        }
        settings.secure_cookies = parse_bool(env.get("FELJOSIT_SECURE_COOKIES"), false);
        if let Some(hosts) = env.get("FELJOSIT_ALLOWED_HOSTS").filter(|v| !v.is_empty()) {
            settings.allowed_hosts =
                hosts.split(',').map(str::trim).filter(|h| !h.is_empty()).map(str::to_string).collect();
        }
        if let Some(proxies) = env.get("FELJOSIT_TRUSTED_PROXIES") {
            settings.trusted_proxies =
                crate::net::parse_networks(proxies).map_err(|e| format!("FELJOSIT_TRUSTED_PROXIES: {e}"))?;
        }
        Ok(settings)
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("feljosit.sqlite3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_from_the_environment() {
        let env: HashMap<String, String> = [
            ("FELJOSIT_DATA_DIR", "/tmp/x"),
            ("FELJOSIT_PORT", "9000"),
            ("FELJOSIT_SECURE_COOKIES", "TRUE"),
            ("FELJOSIT_ALLOWED_HOSTS", "budget.example, localhost ,"),
            ("FELJOSIT_TRUSTED_PROXIES", "172.30.87.1, fd00::/8"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let settings = Settings::from_map(&env).unwrap();
        assert_eq!(settings.data_dir, PathBuf::from("/tmp/x"));
        assert_eq!(settings.port, 9000);
        assert!(settings.secure_cookies);
        assert_eq!(settings.allowed_hosts, vec!["budget.example", "localhost"]);
        assert_eq!(settings.host, "127.0.0.1");
        assert_eq!(settings.trusted_proxies.len(), 2);
    }

    #[test]
    fn defaults() {
        let settings = Settings::from_map(&HashMap::new()).unwrap();
        assert!(settings.data_dir.ends_with("feljosit"));
        assert!(!settings.secure_cookies);
        assert_eq!(settings.port, 8765);
        assert!(settings.trusted_proxies.is_empty());
    }

    #[test]
    fn legacy_directory_with_a_database_is_used() {
        let base = std::env::temp_dir().join(format!("feljosit-config-{}", std::process::id()));
        let (new, old) = (base.join("new"), base.join("old"));
        std::fs::create_dir_all(&old).unwrap();
        assert_eq!(data_dir_with_legacy(new.clone(), Some(old.clone())), new);
        std::fs::write(old.join("feljosit.sqlite3"), b"").unwrap();
        assert_eq!(data_dir_with_legacy(new.clone(), Some(old.clone())), old);
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("feljosit.sqlite3"), b"").unwrap();
        assert_eq!(data_dir_with_legacy(new.clone(), Some(old)), new);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn trusted_proxies_must_be_addresses() {
        let env = HashMap::from([("FELJOSIT_TRUSTED_PROXIES".to_string(), "proxy.home".to_string())]);
        assert!(Settings::from_map(&env).unwrap_err().contains("FELJOSIT_TRUSTED_PROXIES"));
    }

    #[test]
    fn port_must_be_a_number() {
        let env = HashMap::from([("FELJOSIT_PORT".to_string(), "abc".to_string())]);
        assert!(Settings::from_map(&env).is_err());
    }
}
