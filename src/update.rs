//! Checks GitHub for a newer release (the window shows a link if there is one). Only asks
//! GitHub's public API for the latest release; nothing about the user is sent.

use anyhow::Result;

const LATEST: &str = "https://api.github.com/repos/rynmon/bnet-steam-launcher/releases/latest";

/// A newer release than this build: (version, page URL).
pub fn newer_release() -> Result<Option<(String, String)>> {
    let reply = crate::net::get_json(LATEST, &["Accept: application/vnd.github+json".into()])?;
    let tag = reply["tag_name"].as_str().unwrap_or_default();
    let url = reply["html_url"].as_str().unwrap_or_default();
    Ok(
        (is_newer(tag, env!("CARGO_PKG_VERSION")) && !url.is_empty())
            .then(|| (tag.trim_start_matches('v').to_string(), url.to_string())),
    )
}

/// "v1.2.0" > "1.1.9"; pre-release suffixes ("-beta.2") are ignored.
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    let (c, cur) = (parts(candidate), parts(current));
    !candidate.is_empty() && c > cur
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn versions() {
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("v1.10.0", "1.9.3"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.9.0", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
    }
}
