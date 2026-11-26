//! Update check: is there a newer Wisp release on GitHub?
//!
//! The app fetches [`LATEST_RELEASE_PATH`] from [`API_HOST`] and hands the
//! JSON to [`parse_latest`]; nothing is downloaded or installed automatically.

use serde_json::Value;

pub const API_HOST: &str = "api.github.com";
pub const LATEST_RELEASE_PATH: &str = "/repos/Xzpy909/Wisp/releases/latest";
/// Where to send people when the API is unreachable or has no release yet.
pub const RELEASES_PAGE: &str = "https://github.com/Xzpy909/Wisp/releases/latest";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Version without the leading `v`, e.g. `0.2.0`.
    pub version: String,
    /// The release page, where the installer is.
    pub url: String,
}

/// Read the version and page from GitHub's "latest release" response.
pub fn parse_latest(json: &str) -> Option<Release> {
    let v: Value = serde_json::from_str(json).ok()?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = v["tag_name"].as_str()?.trim();
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag).to_string();
    version_parts(&version)?;
    let url = v["html_url"].as_str().filter(|u| u.starts_with("https://")).unwrap_or(RELEASES_PAGE).to_string();
    Some(Release { version, url })
}

/// `1.2.3` → `[1, 2, 3]`; a pre-release suffix (`-beta.1`) is reported separately.
fn version_parts(v: &str) -> Option<(Vec<u64>, bool)> {
    let (core, pre) = match v.split_once('-') {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let nums: Option<Vec<u64>> = core.split('.').map(|p| p.parse().ok()).collect();
    let mut nums = nums.filter(|n| !n.is_empty() && n.len() <= 4)?;
    while nums.len() < 3 {
        nums.push(0);
    }
    Some((nums, pre))
}

/// True when `candidate` is a later version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some((a, a_pre)), Some((b, b_pre))) = (version_parts(candidate), version_parts(current)) else {
        return false;
    };
    match a.cmp(&b) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        // Same numbers: a full release beats its own pre-release.
        std::cmp::Ordering::Equal => !a_pre && b_pre,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(is_newer("1.0.0", "1.0.0-beta.2"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
    }

    #[test]
    fn parses_latest_release() {
        let json = r#"{"tag_name":"v0.3.1","html_url":"https://github.com/Xzpy909/Wisp/releases/tag/v0.3.1","draft":false,"prerelease":false,"assets":[]}"#;
        let r = parse_latest(json).unwrap();
        assert_eq!(r.version, "0.3.1");
        assert_eq!(r.url, "https://github.com/Xzpy909/Wisp/releases/tag/v0.3.1");
    }

    #[test]
    fn ignores_prereleases_and_bad_input() {
        assert_eq!(parse_latest(r#"{"tag_name":"v0.4.0","prerelease":true}"#), None);
        assert_eq!(parse_latest(r#"{"tag_name":"nightly"}"#), None);
        assert_eq!(parse_latest(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_latest("not json"), None);
        // An unexpected URL falls back to the releases page.
        let r = parse_latest(r#"{"tag_name":"0.2.0","html_url":"javascript:alert(1)"}"#).unwrap();
        assert_eq!(r.url, RELEASES_PAGE);
    }
}
