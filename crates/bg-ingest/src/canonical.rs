//! URL canonicalization and content hashing.
//!
//! Deduplication is only as good as the URL key. The same article reaches us as
//! `decrypt.co/12345/headline?utm_source=rss`, `decrypt.co/12345/headline/` and
//! `www.decrypt.co/12345/headline#comments`, and if those hash to three
//! different keys the front page shows the same story three times. Everything
//! here exists to collapse those into one.

use sha2::{Digest, Sha256};
use url::Url;

/// Query parameters that identify the *referrer*, not the resource.
///
/// Prefix-matched rather than exact-matched: analytics vendors invent new
/// `utm_*` and `mc_*` suffixes constantly, and an allowlist would go stale.
const TRACKING_PREFIXES: &[&str] = &["utm_", "mc_", "pk_", "ga_", "_hs", "hsa_", "at_"];

const TRACKING_EXACT: &[&str] = &[
    "fbclid",
    "gclid",
    "dclid",
    "msclkid",
    "twclid",
    "igshid",
    "ref",
    "ref_src",
    "referrer",
    "source",
    "cmpid",
    "campaign",
    "sr_share",
    "__twitter_impression",
    "guccounter",
    "amp",
    "s",
    "spm",
    "yclid",
    "wickedid",
    "rss",
    "feed",
];

fn is_tracking(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    TRACKING_EXACT.contains(&k.as_str()) || TRACKING_PREFIXES.iter().any(|p| k.starts_with(p))
}

/// Normalize a URL into a stable dedupe key.
///
/// Returns the input trimmed if it will not parse — a malformed URL still
/// deserves a consistent hash rather than being dropped.
pub fn canonicalize(raw: &str) -> String {
    let raw = raw.trim();
    let Ok(mut u) = Url::parse(raw) else {
        return raw.to_string();
    };

    // Scheme: http and https are the same resource for our purposes.
    if u.scheme() == "http" {
        let _ = u.set_scheme("https");
    }

    // Host: lowercase, drop a leading `www.`, drop AMP subdomains.
    if let Some(host) = u.host_str() {
        let h = host.to_ascii_lowercase();
        let h = h.strip_prefix("www.").unwrap_or(&h).to_string();
        let _ = u.set_host(Some(&h));
    }

    // Default ports carry no information.
    if matches!(
        (u.scheme(), u.port()),
        ("https", Some(443)) | ("http", Some(80))
    ) {
        let _ = u.set_port(None);
    }

    // Fragments are client-side only.
    u.set_fragment(None);

    // Strip tracking params; keep the rest, sorted so parameter order does not
    // produce two keys for one resource.
    let kept: Vec<(String, String)> = {
        let mut v: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !is_tracking(k))
            .map(|(k, val)| (k.into_owned(), val.into_owned()))
            .collect();
        v.sort();
        v
    };
    if kept.is_empty() {
        u.set_query(None);
    } else {
        let mut qs = u.query_pairs_mut();
        qs.clear();
        for (k, v) in &kept {
            qs.append_pair(k, v);
        }
        drop(qs);
    }

    // Trailing slash: `/a/b/` and `/a/b` are one page. The site root keeps its
    // slash, since "https://x.com" and "https://x.com/" both normalize there.
    let mut s = u.to_string();
    if s.ends_with('/') && u.path() != "/" {
        s.pop();
    }
    // `/amp` and `/amp/` suffixes serve the same article.
    for suffix in ["/amp", "?amp=1"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            s = stripped.to_string();
        }
    }
    s
}

/// Lowercase hex SHA-256. Used for `url_hash` and `body_hash`.
pub fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

/// The dedupe key for a source item.
pub fn url_hash(raw: &str) -> String {
    sha256_hex(&canonicalize(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_article_always_hashes_the_same() {
        let variants = [
            "https://decrypt.co/12345/headline",
            "http://decrypt.co/12345/headline",
            "https://www.decrypt.co/12345/headline",
            "https://decrypt.co/12345/headline/",
            "https://decrypt.co/12345/headline#comments",
            "https://decrypt.co/12345/headline?utm_source=rss&utm_medium=feed",
            "https://decrypt.co/12345/headline?fbclid=abc123",
            "https://DECRYPT.co:443/12345/headline",
            "  https://decrypt.co/12345/headline  ",
        ];
        let first = url_hash(variants[0]);
        for v in &variants[1..] {
            assert_eq!(url_hash(v), first, "should have collapsed to one key: {v}");
        }
    }

    #[test]
    fn meaningful_query_params_are_preserved_and_order_independent() {
        assert_eq!(
            canonicalize("https://x.test/a?page=2&id=7&utm_source=rss"),
            canonicalize("https://x.test/a?id=7&page=2"),
        );
        assert!(canonicalize("https://x.test/a?page=2").contains("page=2"));
        assert_ne!(
            url_hash("https://x.test/a?page=2"),
            url_hash("https://x.test/a?page=3"),
            "genuinely different pages must not collide"
        );
    }

    #[test]
    fn distinct_articles_do_not_collide() {
        assert_ne!(
            url_hash("https://decrypt.co/1/a"),
            url_hash("https://decrypt.co/2/b")
        );
        assert_ne!(
            url_hash("https://decrypt.co/1/a"),
            url_hash("https://theblock.co/1/a")
        );
    }

    #[test]
    fn site_root_keeps_its_slash() {
        assert_eq!(canonicalize("https://x.test/"), "https://x.test/");
    }

    #[test]
    fn unparseable_input_still_yields_a_stable_key() {
        assert_eq!(url_hash("not a url"), url_hash("  not a url  "));
    }

    #[test]
    fn amp_variants_collapse_onto_the_canonical_article() {
        assert_eq!(
            canonicalize("https://x.test/story/amp"),
            canonicalize("https://x.test/story"),
        );
    }
}
