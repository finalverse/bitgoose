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
    let meta = std::fs::metadata(&p).ok()?;
    if !meta.is_file() {
        return None;
    }
    // Never advertise a picture that cannot arrive.
    //
    // The backstop to `fit_for_sharing`, and it earns its place: copies made
    // before that existed are full size, and an image we cannot re-encode is
    // stored as it came. A crawler offered 810 KB over this link gets nothing
    // and renders a blank card — strictly worse than the 14 KB card it would
    // otherwise have been given. Size is the deciding fact, whatever the reason
    // for it.
    (meta.len() as usize <= SHARE_TARGET_BYTES).then_some(p)
}

/// Whether we have already fetched this story's picture, servable or not.
///
/// Distinct from [`mirrored`] on purpose, and the distinction is load-bearing:
///
/// * `mirrored` asks *may we advertise this*, and is size-gated.
/// * `held` asks *have we already done the work*, and is not.
///
/// Without the second, an image that cannot be compressed under the target —
/// 24 of the first 104 — reads as missing forever, so the backfill re-fetches
/// it every round, on the link that is the reason the target exists, for a
/// result it will reject again. One question answering for two produced a
/// permanent retry loop against publishers.
pub fn held(slug: &str) -> bool {
    safe_slug(slug) && cache_dir().join(format!("img-{slug}")).is_file()
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
    let out = fit_for_sharing(&bytes);
    let stored = out.len();
    store(&cache_dir().join(format!("img-{slug}")), &out);
    info!(
        %slug,
        from = bytes.len(),
        to = stored,
        "mirrored the publisher's lead image"
    );
    true
}

/// Widest a share image ever needs to be.
///
/// Every platform crops from around 1200x630, and WeChat renders its thumbnail
/// at roughly a hundred pixels. Anything larger is bytes nobody sees.
const SHARE_WIDTH: u32 = 1200;

/// What a share image must fit inside to actually arrive.
///
/// Not an aesthetic limit — a transport one. Measured against production: a
/// 146 KB photograph took **28 seconds** to fetch and timed out at every budget
/// under ten, while the 14 KB card we draw arrived every time. Mirroring
/// publishers' images at their own resolution therefore made previews *worse*
/// than the card it replaced, on 80 of the first 91 copied. The median was
/// 174 KB and the largest 810 KB.
/// Measured again after the first attempt at this, because 60,000 was still a
/// guess dressed as a limit. At the ~7 KB/s this link actually delivers:
///
/// ```text
///   14 KB card    4.5s
///   44 KB photo   6.1s
///  120 KB photo  17.9s
/// ```
///
/// Nothing here makes a two-second crawler budget, including the card — that is
/// the unplugged ethernet and no encoder setting fixes it. What the target
/// *can* decide is that a photograph is never meaningfully worse than the card
/// it replaces. 45 KB puts it within a second or two of one.
const SHARE_TARGET_BYTES: usize = 45_000;

/// Re-encode a publisher's image at the size a share card actually uses.
///
/// Returns the original untouched if it is already small enough, or if it
/// cannot be decoded — a picture we cannot re-encode is still better than none,
/// and the caller's size guard is the backstop.
pub fn fit_for_sharing(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() <= SHARE_TARGET_BYTES {
        return bytes.to_vec();
    }
    let Ok(img) = image::load_from_memory(bytes) else {
        return bytes.to_vec();
    };
    let img = if img.width() > SHARE_WIDTH {
        img.resize(
            SHARE_WIDTH,
            u32::MAX,
            image::imageops::FilterType::CatmullRom,
        )
    } else {
        img
    };
    // A ladder, not two guesses.
    //
    // Two passes were tried and measured against a real rejected photograph:
    // 1200x800 at q74 gave 151,743 bytes and 800x533 at q62 gave 64,466 — both
    // over, so the original came back untouched and **97 of 206 copies were
    // full size**. There was no error; the passes were simply not aggressive
    // enough, which a fixed pair of settings cannot know.
    //
    // So it steps down until it fits. The floor is 560px, which is still five
    // times WeChat's rendered thumbnail — beyond that the picture would be
    // getting worse for no one's benefit.
    for (width, quality) in [(SHARE_WIDTH, 74u8), (900, 66), (700, 58), (560, 50)] {
        let scaled = if img.width() > width {
            img.resize(width, u32::MAX, image::imageops::FilterType::CatmullRom)
        } else {
            img.clone()
        };
        let mut out = Vec::with_capacity(SHARE_TARGET_BYTES);
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        if scaled.to_rgb8().write_with_encoder(enc).is_err() {
            continue;
        }
        if out.len() <= SHARE_TARGET_BYTES {
            return out;
        }
    }
    // Even at the floor it will not fit — a very large or very noisy image.
    // Handed back unchanged so `mirrored` declines to advertise it and the
    // story falls back to the card, which does arrive. Smaller was never the
    // bar; arriving is.
    bytes.to_vec()
}

#[cfg(test)]
mod share_size_tests {
    use super::*;

    fn photo(w: u32, h: u32) -> Vec<u8> {
        // Smooth gradients with a little structure — what a JPEG is built for,
        // and what a news photograph mostly is. An earlier version of this
        // fixture was high-frequency noise, which no encoder can squeeze under
        // the target; the test then failed for being unrepresentative rather
        // than for anything being wrong.
        let mut img = image::RgbImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let r = (fx * 220.0) as u8;
            let g = (fy * 200.0 + 30.0) as u8;
            let b = (((fx + fy) * 0.5) * 180.0 + 40.0) as u8;
            *p = image::Rgb([r, g, b]);
        }
        let mut out = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 96);
        image::DynamicImage::ImageRgb8(img)
            .to_rgb8()
            .write_with_encoder(enc)
            .unwrap();
        out
    }

    #[test]
    fn a_publishers_full_size_photograph_is_cut_down() {
        // The regression this exists to prevent: a 146 KB image took 28
        // seconds over the production link and timed out at every crawler
        // budget under ten.
        let big = photo(2400, 1350);
        assert!(
            big.len() > SHARE_TARGET_BYTES,
            "fixture is too small to test"
        );
        let out = fit_for_sharing(&big);
        assert!(
            out.len() <= SHARE_TARGET_BYTES,
            "{} bytes in, {} out, target {}",
            big.len(),
            out.len(),
            SHARE_TARGET_BYTES
        );
        let img = image::load_from_memory(&out).expect("still a valid image");
        assert!(img.width() <= SHARE_WIDTH, "width {}", img.width());
    }

    /// A photograph at the size publishers actually serve.
    ///
    /// The two-pass version measured 151,743 then 64,466 bytes on a real
    /// 1500x1000 news photograph and gave up, leaving it full size. The ladder
    /// gets the same picture to about 31 KB.
    #[test]
    fn a_typical_press_photograph_reaches_the_target() {
        let big = photo(1500, 1000);
        let out = fit_for_sharing(&big);
        assert!(
            out.len() <= SHARE_TARGET_BYTES,
            "{} bytes, target {}",
            out.len(),
            SHARE_TARGET_BYTES
        );
        let img = image::load_from_memory(&out).expect("still a valid image");
        // Not squeezed past usefulness: WeChat renders this near 110px.
        assert!(img.width() >= 560, "shrunk to {}px", img.width());
    }

    #[test]
    fn something_already_small_is_left_alone() {
        let small = photo(400, 300);
        if small.len() <= SHARE_TARGET_BYTES {
            assert_eq!(fit_for_sharing(&small), small, "re-encoded needlessly");
        }
    }

    #[test]
    fn undecodable_bytes_pass_through_rather_than_vanish() {
        // A picture we cannot re-encode is still better than none, and the
        // caller's size guard is the backstop.
        let junk = vec![0xAB; SHARE_TARGET_BYTES + 500];
        assert_eq!(fit_for_sharing(&junk).len(), junk.len());
    }
}

#[cfg(test)]
mod arrival_tests {
    use super::*;

    /// The bar is arriving, not shrinking.
    ///
    /// A 119,865-byte file was stored by an earlier version of
    /// `fit_for_sharing` because it was smaller than its 400 KB original. It
    /// takes eighteen seconds to fetch over the production link, where the card
    /// it replaced takes four.
    #[test]
    fn a_result_that_misses_the_target_is_not_kept() {
        // Photographic noise at a size no JPEG setting will squeeze under the
        // target, so the second pass is guaranteed to miss.
        let mut img = image::RgbImage::new(3000, 2000);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let n = (x
                .wrapping_mul(2654435761)
                .wrapping_add(y.wrapping_mul(40503))) as u8;
            *p = image::Rgb([n, n.rotate_left(3), n.wrapping_mul(7)]);
        }
        let mut original = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut original, 98);
        img.write_with_encoder(enc).unwrap();

        let out = fit_for_sharing(&original);
        // Either it met the target, or it came back untouched for `mirrored`
        // to reject. What it must never be is "smaller, but still too big".
        assert!(
            out.len() <= SHARE_TARGET_BYTES || out == original,
            "kept a {}-byte result that misses the {}-byte target",
            out.len(),
            SHARE_TARGET_BYTES
        );
    }
}

#[cfg(test)]
mod held_tests {
    use super::*;

    /// The two questions must not collapse into one.
    ///
    /// A picture too large to advertise is still a picture we fetched. Asking
    /// `mirrored` in place of `held` made the backfill re-download 24 files
    /// every round, permanently, over the link the size limit exists because of.
    #[test]
    fn a_file_too_large_to_serve_is_still_a_file_we_have() {
        let slug = "held-test-oversized-story-slug";
        let path = cache_dir().join(format!("img-{slug}"));
        store(&path, &vec![0u8; SHARE_TARGET_BYTES + 5_000]);

        assert!(held(slug), "we fetched it, so we hold it");
        assert!(mirrored(slug).is_none(), "but it must not be advertised");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn something_never_fetched_is_neither() {
        assert!(!held("held-test-a-slug-we-have-never-seen"));
        assert!(mirrored("held-test-a-slug-we-have-never-seen").is_none());
    }
}
