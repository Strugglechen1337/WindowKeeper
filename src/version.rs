//! Version parsing and comparison for release tags.

use semver::Version;

/// Parse a version string with an optional `v`/`V` prefix.
pub fn parse_tag(tag: &str) -> Option<Version> {
    let t = tag.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    Version::parse(t).ok()
}

/// True if `latest` is a strictly newer version than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_tag(current), parse_tag(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_is_optional() {
        assert_eq!(parse_tag("v1.2.3"), Version::parse("1.2.3").ok());
        assert_eq!(parse_tag("1.2.3"), parse_tag("V1.2.3"));
        assert!(parse_tag("latest").is_none());
    }

    #[test]
    fn comparison() {
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(!is_newer("0.1.0", "v0.1.0"));
        assert!(!is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("1.0.0", "garbage"));
        assert!(is_newer("1.0.0-beta.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0-rc.1"));
    }
}
