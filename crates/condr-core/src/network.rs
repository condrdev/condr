//! `[network]`: how every Condr process on this machine reaches out, the Server's
//! Peer-to-peer relay and the GUI's update check alike (ADR 0038).

use std::path::Path;

/// `[network.proxy] mode`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProxyMode {
    /// The `HTTPS_PROXY` family and `NO_PROXY`, then the operating system's own settings.
    #[default]
    System,
    None,
    /// `[network.proxy] url`.
    Manual,
}

impl ProxyMode {
    pub const ALL: [Self; 3] = [Self::System, Self::None, Self::Manual];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::None => "none",
            Self::Manual => "manual",
        }
    }

    /// Unset or unrecognized values are `System`, the default.
    pub fn parse(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value)
            .unwrap_or_default()
    }
}

/// `[network.proxy]`. The URL is kept while another mode is chosen, so going back to
/// Manual finds it again.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProxySetting {
    pub mode: ProxyMode,
    pub url: String,
}

impl ProxySetting {
    /// The saved setting; a missing or malformed file is the default.
    pub fn load(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let read = |key| {
            crate::read_config_value(path, &["network", "proxy"], key)
                .ok()
                .flatten()
                .and_then(|value| value.as_str().map(|value| value.trim().to_owned()))
                .unwrap_or_default()
        };
        Self {
            mode: ProxyMode::parse(&read("mode")),
            url: read("url"),
        }
    }

    /// The proxy Manual names; a blank or invalid URL means none.
    pub fn manual_url(&self) -> Option<url::Url> {
        if self.mode != ProxyMode::Manual || check_proxy_url(&self.url).is_err() {
            return None;
        }
        self.url.parse().ok()
    }
}

/// Whether `url` can be `[network.proxy] url`: blank, or `http://` or `https://` with a
/// host, the proxies iroh's relay client can speak HTTP CONNECT to.
pub fn check_proxy_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(());
    }
    match url::Url::parse(url) {
        Ok(parsed) if !matches!(parsed.scheme(), "http" | "https") => {
            Err("Only http:// and https:// proxies are supported".into())
        }
        Ok(parsed) if parsed.host_str().is_some_and(|host| !host.is_empty()) => Ok(()),
        _ => Err("Enter a URL such as http://proxy:8080".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https_urls_with_a_host_are_accepted() {
        for url in ["", "  ", "http://proxy:8080", "https://user:pass@proxy"] {
            assert_eq!(check_proxy_url(url), Ok(()), "{url}");
        }
        for url in ["socks5://proxy:1080", "proxy:8080", "http://", "not a url"] {
            assert!(check_proxy_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn the_setting_round_trips_and_falls_back_to_system() {
        let directory = std::env::temp_dir().join(format!(
            "condr-core-network-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(ProxySetting::load(Some(&path)), ProxySetting::default());

        crate::update_config_values(
            &path,
            &["network", "proxy"],
            [
                ("mode", Some(toml_edit::value("manual"))),
                ("url", Some(toml_edit::value(" http://proxy:8080 "))),
            ],
        )
        .unwrap();
        let setting = ProxySetting::load(Some(&path));
        assert_eq!(setting.mode, ProxyMode::Manual);
        assert_eq!(
            setting.manual_url().map(String::from).as_deref(),
            Some("http://proxy:8080/")
        );

        crate::update_config_values(
            &path,
            &["network", "proxy"],
            [("mode", Some(toml_edit::value("socks")))],
        )
        .unwrap();
        let setting = ProxySetting::load(Some(&path));
        assert_eq!(setting.mode, ProxyMode::System);
        assert_eq!(setting.url, "http://proxy:8080", "the URL is kept");
        assert_eq!(setting.manual_url(), None);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
