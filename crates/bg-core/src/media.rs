//! Image URL normalisation.
//!
//! Feeds hand us URLs that are not images. YouTube's `media:content` carries
//! the *player* URL — `https://www.youtube.com/v/<id>?version=3` — and putting
//! that in an `<img src>` produces a permanently broken image, which is what
//! four stories on the front page were doing. The video id inside it is enough
//! to build the real thumbnail, so nothing has to be re-fetched.
//!
//! Pure string work, no I/O: this crate compiles to WASM and the hydrate bundle
//! calls it too.

/// YouTube's thumbnail for a video id.
///
/// `hqdefault` rather than `maxresdefault` because the latter only exists for
/// videos uploaded above 720p — asking for it trades one broken image for
/// another on older uploads.
pub fn youtube_thumb(video_id: &str) -> String {
    format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg")
}

/// The video id inside any YouTube URL shape we actually receive.
pub fn youtube_id(url: &str) -> Option<&str> {
    let rest = url
        .split_once("youtube.com/v/")
        .or_else(|| url.split_once("youtube.com/embed/"))
        .or_else(|| url.split_once("youtu.be/"))
        .map(|(_, r)| r)
        .or_else(|| {
            url.split_once("youtube.com/watch?v=")
                .map(|(_, r)| r)
                .or_else(|| url.split_once("&v=").map(|(_, r)| r))
        })?;
    let id = rest
        .split(['?', '&', '/', '#'])
        .next()
        .unwrap_or_default()
        .trim();
    // Reject anything that is not plausibly an id, so a malformed URL becomes
    // "no image" rather than a link to a 404 on someone else's CDN.
    (!id.is_empty()
        && id.len() <= 24
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then_some(id)
}

/// Make `url` safe to put in an `<img src>`, or return `None` if it cannot be.
///
/// Only YouTube is special-cased, because it is the only source in the roster
/// that syndicates a non-image where an image belongs. Everything else passes
/// through untouched — guessing at other publishers' CDN conventions would
/// break more than it fixed.
pub fn as_image(url: &str) -> Option<String> {
    let u = url.trim();
    if u.is_empty() {
        return None;
    }
    if u.contains("youtube.com/") || u.contains("youtu.be/") {
        return youtube_id(u).map(youtube_thumb);
    }
    Some(u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_player_url_a_feed_gives_us_becomes_a_real_thumbnail() {
        // Exactly what was rendering broken on the front page.
        assert_eq!(
            as_image("https://www.youtube.com/v/_4B2-k5OaDw?version=3").as_deref(),
            Some("https://i.ytimg.com/vi/_4B2-k5OaDw/hqdefault.jpg")
        );
    }

    #[test]
    fn every_youtube_url_shape_yields_the_same_id() {
        for u in [
            "https://www.youtube.com/v/dQw4w9WgXcQ?version=3",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_id(u), Some("dQw4w9WgXcQ"), "{u}");
        }
    }

    #[test]
    fn a_youtube_url_with_no_readable_id_yields_no_image() {
        // Better than pointing an <img> at a channel page.
        assert_eq!(as_image("https://www.youtube.com/"), None);
        assert_eq!(as_image("https://www.youtube.com/v/?version=3"), None);
    }

    #[test]
    fn other_publishers_are_left_alone() {
        let u = "https://cdn.example.com/a/b.jpg?width=1200";
        assert_eq!(as_image(u).as_deref(), Some(u));
        assert_eq!(as_image("   "), None);
    }
}
