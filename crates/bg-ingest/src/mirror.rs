//! Our own copy of a publisher's lead image.
//!
//! Lives here rather than in the web crate because **the fetch has to happen at
//! publish time**, not on the first crawler request. The first version warmed
//! the cache when a preview was first asked for — but a preview client caches
//! what it got, and WeChat caches per URL forever, so the first share of every
//! story permanently showed the generated card even for stories with a
//! photograph sitting one fetch away. The newsroom publishes; the newsroom
//! should fetch.
//!
//! The web crate serves what is here; nothing else reads it.

use std::path::PathBuf;
use tracing::{debug, info};

/// Where rendered cards and mirrored images live between restarts.
///
/// The point of putting them on disk at all is that a restart must not send the
/// next crawler back through an 8-second render. If the configured directory
/// cannot be created we fall back to the temp dir rather than failing: a
/// slower cache is still a cache, and an unwritable `/var/cache` should not
/// take the site down.
pub fn cache_dir() -> &'static PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let want = std::env::var("BG_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/var/cache/bitgoose/assets"));
        if std::fs::create_dir_all(&want).is_ok() {
            return want;
        }
        let fallback = std::env::temp_dir().join("bitgoose-assets");
        tracing::warn!(
            path = %want.display(),
            using = %fallback.display(),
            "cache directory is not writable; share assets will not survive a reboot"
        );
        let _ = std::fs::create_dir_all(&fallback);
        fallback
    })
}

/// Write through a temporary file and rename.
///
/// A crawler reading a half-written PNG gets a corrupt image and caches the
/// result, which outlives the race by however long its cache does. Rename is
/// atomic within a filesystem, so a reader sees either the old file or the
/// whole new one.
pub fn store(path: &std::path::Path, bytes: &[u8]) {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Slugs are used as filenames, so they are held to what a slug actually is.
///
/// Not defence in depth — the DB lookup already constrains this to slugs we
/// published — but a path is being built from the value, and a component that
/// builds a path should not take the caller's word for its shape.
pub fn safe_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 200
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Content type from the leading bytes. `None` means it is not an image we
/// recognise, and we will not serve it.
///
/// Trusts the bytes, not the header: a `Content-Type` of `image/jpeg` on an
/// HTML error page is common, and this gets served to every reader.
pub fn sniff(b: &[u8]) -> Option<&'static str> {
    match b {
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        _ if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" => Some("image/webp"),
        // SVG can carry script and we would be serving it same-origin.
        _ if b.starts_with(b"<svg") || b.starts_with(b"<?xml") => None,
        _ => None,
    }
}

/// Path of the mirrored image for a story, if we hold one.
///
/// The page loader calls this to decide what to advertise. Advertising a
/// picture we have not fetched yet puts the fetch on the crawler's clock, which
/// is the failure the whole cache exists to remove.
pub fn mirrored(slug: &str) -> Option<PathBuf> {
    if !safe_slug(slug) {
        return None;
    }
    let p = cache_dir().join(format!("img-{slug}"));
    p.is_file().then_some(p)
}

/// Largest publisher image worth storing. Above this it is not a lead image.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// Fetch a story's lead image and keep our own copy.
///
/// Returns whether we now hold one. Idempotent and cheap when already held, so
/// it is safe to call on every publish.
pub async fn store_lead_image(client: &reqwest::Client, slug: &str, url: &str) -> bool {
    if !safe_slug(slug) {
        return false;
    }
    if mirrored(slug).is_some() {
        return true;
    }
    let Ok(resp) = client.get(url).send().await else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    let Ok(bytes) = resp.bytes().await else {
        return false;
    };
    if bytes.len() > MAX_IMAGE_BYTES || sniff(&bytes).is_none() {
        debug!(%url, bytes = bytes.len(), "not a usable image; keeping our own card");
        return false;
    }
    store(&cache_dir().join(format!("img-{slug}")), &bytes);
    info!(%slug, bytes = bytes.len(), "mirrored the publisher's lead image");
    true
}
