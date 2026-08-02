//! Recognising syndicated video.
//!
//! Only the provider's opaque id is kept, never a URL and never markup. The
//! embed host is then our choice at render time, so a feed cannot dictate what
//! ends up in an iframe `src` — the difference between "the publisher told us
//! which video this is" and "the publisher told us what to load".

/// Extract a YouTube video id from a feed entry's link or id.
///
/// YouTube channel feeds give both: entry ids look like `yt:video:<ID>` and the
/// alternate link is a normal watch URL. Either is accepted, since a feed that
/// changes one rarely changes both.
///
/// Returns `None` for anything that is not a plain 11-character YouTube id, so
/// a malformed or hostile value is dropped rather than stored.
pub fn youtube_id(link: &str, entry_id: &str) -> Option<String> {
    let candidate = from_entry_id(entry_id).or_else(|| from_url(link))?;
    valid(&candidate).then_some(candidate)
}

fn from_entry_id(entry_id: &str) -> Option<String> {
    entry_id
        .strip_prefix("yt:video:")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn from_url(link: &str) -> Option<String> {
    let rest = link
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(link)
        .trim_start_matches("www.");

    // youtu.be/<id>
    if let Some(r) = rest.strip_prefix("youtu.be/") {
        return Some(stop_at_delimiter(r));
    }
    // youtube.com/watch?v=<id>, /embed/<id>, /shorts/<id>, /live/<id>
    let path = rest.strip_prefix("youtube.com")?;
    for prefix in ["/embed/", "/shorts/", "/live/", "/v/"] {
        if let Some(r) = path.strip_prefix(prefix) {
            return Some(stop_at_delimiter(r));
        }
    }
    let query = path.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("v="))
        .map(stop_at_delimiter)
}

fn stop_at_delimiter(s: &str) -> String {
    s.chars()
        .take_while(|c| !matches!(c, '?' | '&' | '/' | '#'))
        .collect()
}

/// YouTube ids are exactly 11 characters of an unreserved alphabet. Anything
/// else is not one, and must not reach an iframe.
fn valid(id: &str) -> bool {
    id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_id_from_either_field() {
        assert_eq!(
            youtube_id(
                "https://www.youtube.com/watch?v=s3Ibar7yYS4",
                "yt:video:s3Ibar7yYS4"
            )
            .as_deref(),
            Some("s3Ibar7yYS4")
        );
        // Link alone, when the id field is something else.
        assert_eq!(
            youtube_id(
                "https://www.youtube.com/watch?v=s3Ibar7yYS4",
                "tag:example,2026:1"
            )
            .as_deref(),
            Some("s3Ibar7yYS4")
        );
    }

    #[test]
    fn handles_the_url_shapes_youtube_actually_emits() {
        for url in [
            "https://youtu.be/s3Ibar7yYS4",
            "https://www.youtube.com/embed/s3Ibar7yYS4",
            "https://www.youtube.com/shorts/s3Ibar7yYS4",
            "https://www.youtube.com/live/s3Ibar7yYS4",
            "https://youtube.com/watch?feature=share&v=s3Ibar7yYS4",
            "https://www.youtube.com/watch?v=s3Ibar7yYS4&t=42s",
            "https://youtu.be/s3Ibar7yYS4?si=abcd",
        ] {
            assert_eq!(youtube_id(url, "").as_deref(), Some("s3Ibar7yYS4"), "{url}");
        }
    }

    #[test]
    fn rejects_anything_that_is_not_an_id() {
        // Non-YouTube hosts, and lookalike hosts.
        assert_eq!(youtube_id("https://thedefiant.io/news/x", "tag:1"), None);
        assert_eq!(
            youtube_id("https://notyoutube.com/watch?v=s3Ibar7yYS4", ""),
            None
        );
        // Wrong length.
        assert_eq!(youtube_id("https://youtu.be/short", ""), None);
        assert_eq!(
            youtube_id("https://youtu.be/waaaaaaaaaaaaaytoolong", ""),
            None
        );
        // An attempt to break out of the embed URL: the delimiter stops it, and
        // the length check then rejects what is left.
        assert_eq!(
            youtube_id("https://www.youtube.com/embed/abc\"></iframe><script>", ""),
            None
        );
        assert_eq!(youtube_id("", ""), None);
    }

    #[test]
    fn an_id_never_carries_markup_or_separators() {
        for hostile in [
            "yt:video:abc/../../evil",
            "yt:video:ab cdefghijk",
            "yt:video:<script>xx",
        ] {
            assert_eq!(youtube_id("", hostile), None, "{hostile}");
        }
    }
}
