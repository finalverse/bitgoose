//! URL slug generation.
//!
//! Slugs are permanent public identifiers — once a story is out, the URL must
//! not drift when a correction rewrites the headline. Generation is therefore
//! deterministic and callers persist the result rather than recomputing it.

/// ASCII-lowercase, hyphen-separated slug, truncated on a word boundary.
pub fn slugify(s: &str) -> String {
    slugify_max(s, 72)
}

pub fn slugify_max(s: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true; // suppresses a leading dash
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if c == '&' {
            // Spelled out rather than dropped: "Coinbase & Circle" must not
            // collapse to "coinbase-circle", which reads as a single entity.
            if !last_dash {
                out.push('-');
            }
            out.push_str("and-");
            last_dash = true;
        } else if matches!(c, ' ' | '-' | '_' | '/' | '.' | ',' | ':' | ';') || c.is_whitespace() {
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else if !c.is_ascii() {
            // Transliterate the accented Latin we actually see in crypto
            // coverage; drop anything else rather than emit mojibake.
            if let Some(rep) = transliterate(c) {
                out.push_str(rep);
                last_dash = false;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() > max_len {
        let cut = out[..max_len].rfind('-').unwrap_or(max_len);
        out.truncate(cut);
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out.is_empty() {
        out.push_str("story");
    }
    out
}

fn transliterate(c: char) -> Option<&'static str> {
    Some(match c.to_ascii_lowercase() {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => "a",
        'é' | 'è' | 'ê' | 'ë' => "e",
        'í' | 'ì' | 'î' | 'ï' => "i",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => "o",
        'ú' | 'ù' | 'û' | 'ü' => "u",
        'ñ' => "n",
        'ç' => "c",
        'ß' => "ss",
        'ø' => "o",
        'æ' => "ae",
        _ => return None,
    })
}

/// Appends a short disambiguator when a slug already exists.
pub fn slug_with_suffix(base: &str, n: u32) -> String {
    format!("{base}-{n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_slugs() {
        assert_eq!(
            slugify("SEC Approves Spot Ether ETFs"),
            "sec-approves-spot-ether-etfs"
        );
        assert_eq!(
            slugify("  Bitcoin's $100,000 Day!  "),
            "bitcoins-100-000-day"
        );
        // Apostrophes close up rather than splitting: "whats", not "what-s".
        assert_eq!(slugify("DeFi / NFT: what's next?"), "defi-nft-whats-next");
    }

    #[test]
    fn never_empty_never_edge_dashes() {
        assert_eq!(slugify("!!!"), "story");
        assert_eq!(slugify(""), "story");
        assert!(!slugify("--hello--").starts_with('-'));
        assert!(!slugify("--hello--").ends_with('-'));
    }

    #[test]
    fn truncates_on_a_word_boundary() {
        let long = "a very long headline that keeps going and going and going past the limit";
        let s = slugify_max(long, 20);
        assert!(s.len() <= 20, "got {} chars: {s}", s.len());
        assert!(!s.ends_with('-'));
    }

    #[test]
    fn transliterates_rather_than_dropping() {
        assert_eq!(slugify("Café & Bär"), "cafe-and-bar");
    }
}
