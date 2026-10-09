//! Talos version strings as TCS stores and compares them.
//!
//! Nodes report `Tag: v1.14.2` or sometimes a bare `1.14.2`. Callers that
//! decide "is the cluster on this version?" must treat those as the same
//! string, and must not treat a half-upgraded fleet as a single version.

/// Canonical form: empty when the value is missing or a placeholder, otherwise
/// a single leading `v` plus the rest of the tag (`1.14.2` and `v1.14.2` both
/// become `v1.14.2`).
pub fn normalize_talos_version(v: &str) -> String {
    let t = v.trim();
    if t.is_empty() || t == "0.0.0" || t.eq_ignore_ascii_case("v0.0.0") {
        return String::new();
    }
    let rest = t.strip_prefix('v').or_else(|| t.strip_prefix('V')).unwrap_or(t);
    format!("v{rest}")
}

/// The version every non-empty entry agrees on, or `None` when the set is
/// empty or mixed. Blank and `0.0.0` placeholders are ignored so one node
/// that has not reported yet does not hide a unanimous fleet.
pub fn consensus_talos_version<I, S>(versions: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut agreed: Option<String> = None;
    for v in versions {
        let n = normalize_talos_version(v.as_ref());
        if n.is_empty() {
            continue;
        }
        match &agreed {
            None => agreed = Some(n),
            Some(cur) if cur == &n => {}
            Some(_) => return None,
        }
    }
    agreed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_optional_v_prefix() {
        assert_eq!(normalize_talos_version("v1.14.2"), "v1.14.2");
        assert_eq!(normalize_talos_version("1.14.2"), "v1.14.2");
        assert_eq!(normalize_talos_version("  V1.14.2 "), "v1.14.2");
        assert_eq!(normalize_talos_version(""), "");
        assert_eq!(normalize_talos_version("0.0.0"), "");
        assert_eq!(normalize_talos_version("v0.0.0"), "");
    }

    #[test]
    fn consensus_requires_unanimous_real_versions() {
        assert_eq!(consensus_talos_version(["v1.14.2", "1.14.2"]), Some("v1.14.2".into()));
        assert_eq!(
            consensus_talos_version(["v1.14.2", "", "0.0.0"]),
            Some("v1.14.2".into())
        );
        assert_eq!(consensus_talos_version(["v1.13.7", "v1.14.2"]), None);
        assert_eq!(consensus_talos_version::<[&str; 0], &str>([]), None);
        assert_eq!(consensus_talos_version(["", "0.0.0"]), None);
    }
}
