//! Generated share cards.
//!
//! Every story needs a picture when it is shared, and most do not have one:
//! a wire item from a text-only feed, an arXiv preprint, a newsletter. Falling
//! back to one static image for all of them means a timeline full of identical
//! BitGoose logos, which reads as a bot rather than a newsroom — and tells a
//! reader nothing about what they are being offered.
//!
//! So a story with no usable publisher image gets its own card: the headline
//! set large, the desk it came from, how many sources back it, on the house
//! palette. That is a picture of *this* story, generated from facts we already
//! hold.
//!
//! Rendered with resvg, which is pure Rust — no ImageMagick, no headless
//! browser, nothing to install on the host.
//!
//! **Text needs a real font**, and fonts are the one part that cannot be
//! guaranteed. We load whatever the system has and fall back to the static card
//! if it has nothing usable, because a generic picture beats a card of blank
//! rectangles where the headline should be.

use std::sync::OnceLock;

/// Open Graph's large-card size. X, LinkedIn and Facebook all render 1200x630
/// as a full-bleed card; anything smaller degrades to a thumbnail.
pub const W: u32 = 1200;
pub const H: u32 = 630;

/// WeChat is the exception, and the reason this module has two shapes.
///
/// A link posted in a WeChat chat renders as a **small square thumbnail** beside
/// the title, centre-cropped from whatever `og:image` provides. Feed a 1200x630
/// card into that and the crop takes a horizontal band out of the middle —
/// half a line of headline, no wordmark, no context. Which is why a Reuters
/// link shows the Reuters roundel and ours showed nothing worth showing.
///
/// So WeChat is served a square card built for the crop it is going to perform.
/// The story is identical; only the geometry differs, the same way a newspaper
/// sets a different crop for the front page and the app.
pub const SQ: u32 = 800;

/// Which card to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// 1200x630 — X, LinkedIn, Facebook, Slack, iMessage.
    Wide,
    /// 800x800 — WeChat, and anywhere else that crops to a square.
    Square,
}

impl Shape {
    pub fn size(self) -> (u32, u32) {
        match self {
            Self::Wide => (W, H),
            Self::Square => (SQ, SQ),
        }
    }
}

/// System fonts, loaded once.
///
/// `None` means the host has no usable font and the caller should serve the
/// static card instead.
fn fonts() -> Option<&'static resvg::usvg::fontdb::Database> {
    static DB: OnceLock<Option<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        if db.is_empty() {
            tracing::warn!(
                "no system fonts found; generated share cards will fall back to the static one"
            );
            return None;
        }
        Some(db)
    })
    .as_ref()
}

/// Which desk a story is on, for the card's accent.
fn accent(beat: &str) -> &'static str {
    match beat {
        "ai" => "#7aa2f7",
        "crypto" => "#f5b301",
        "markets" => "#3fbf7f",
        "tech" => "#bb9af7",
        _ => "#f5b301",
    }
}

/// The goose mark as a path, at a given origin and scale.
///
/// The same geometry as `public/favicon.svg`, drawn on a 64-unit grid. A share
/// card without the mark is a rectangle of text that could belong to anyone —
/// and on the platforms that crop hardest, the mark is the only part of the
/// card that survives at thumbnail size.
fn mark(x: f32, y: f32, size: f32, accent: &str) -> String {
    let k = size / 64.0;
    format!(
        concat!(
            // `r#"…"#` will not do here: the fill colours contain `"#`, which
            // closes the literal early.
            r##"<g transform="translate({} {}) scale({})">"##,
            r##"<path fill="{}" d="M22 56C20 44 21 34 26 27C30 21 36 18 41 19C47 20 52 25 52 32"##,
            r##"L62 34L51 41C47 45 41 46 38 45C34 44 31 41 30 37C29 42 29 49 30 56Z"/>"##,
            r##"<circle cx="42" cy="28" r="2.6" fill="#0b0d10"/></g>"##,
        ),
        x, y, k, accent
    )
}

/// Escape for XML text content. A headline containing `&` or `<` would
/// otherwise produce a document that fails to parse, and the card would
/// silently become the static fallback.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Break a headline into display lines.
///
/// Wrapping by character count rather than measured width: resvg has no layout
/// API we can query before rendering, and for a known font size on a known
/// canvas the approximation is close enough that the alternative — one long
/// line running off the edge — is the only outcome worth avoiding.
fn wrap(text: &str, per_line: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur = word.to_string();
        } else if cur.chars().count() + 1 + word.chars().count() <= per_line {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
            if lines.len() == max_lines {
                break;
            }
        }
    }
    if lines.len() < max_lines && !cur.is_empty() {
        lines.push(cur);
    }
    // A headline cut mid-sentence should say so rather than just stopping.
    if lines.len() == max_lines {
        let used: usize = lines.iter().map(|l| l.chars().count() + 1).sum();
        if used < text.chars().count() {
            if let Some(last) = lines.last_mut() {
                last.push('…');
            }
        }
    }
    lines
}

/// What the card says about a story.
pub struct Card<'a> {
    pub headline: &'a str,
    pub beat: &'a str,
    pub section: &'a str,
    pub sources: i32,
    /// Shown only when the Skein had something to say — it is the reason to
    /// click, so it belongs on the card that advertises the story.
    pub has_analysis: bool,
}

/// Build the card as SVG.
///
/// Separate from rasterising so the layout can be tested without fonts or a
/// renderer.
pub fn svg(card: &Card<'_>) -> String {
    shaped(card, Shape::Wide)
}

/// Build the card as SVG at a given shape.
pub fn shaped(card: &Card<'_>, shape: Shape) -> String {
    if shape == Shape::Square {
        return square(card);
    }
    let accent = accent(card.beat);
    // Longer headlines get set smaller so they still fit three lines. The
    // thresholds are where 3 lines stops being enough at the larger size.
    let (size, per_line) = match card.headline.chars().count() {
        0..=60 => (62.0_f32, 26),
        61..=110 => (52.0, 32),
        _ => (44.0, 38),
    };
    let lines = wrap(card.headline, per_line, 3);

    let mut tspans = String::new();
    for (i, line) in lines.iter().enumerate() {
        // Absolute x on every line: a tspan inheriting x from the parent would
        // continue the previous line rather than start under it.
        tspans.push_str(&format!(
            r#"<tspan x="80" dy="{}">{}</tspan>"#,
            if i == 0 { 0.0 } else { size * 1.22 },
            esc(line)
        ));
    }

    let n = card.sources.max(1);
    let mut footer = format!("{n} source{}", if n == 1 { "" } else { "s" });
    if card.has_analysis {
        footer.push_str("  ·  Includes BitGoose analysis");
    }

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">
  <rect width="{W}" height="{H}" fill="#0b0d10"/>
  <rect x="0" y="0" width="10" height="{H}" fill="{accent}"/>
  <g font-family="Ubuntu, DejaVu Sans, Liberation Sans, Arial, sans-serif">
    {mark}
    <text x="150" y="96" font-size="26" font-weight="700" fill="{accent}"
          letter-spacing="4">BITGOOSE</text>
    <text x="80" y="96" font-size="22" fill="#838c97" letter-spacing="3"
          text-anchor="end" transform="translate({label_x} 0)">{section}</text>
    <text x="80" y="250" font-size="{size}" font-weight="700" fill="#edeae3">{tspans}</text>
    <text x="80" y="556" font-size="24" fill="#838c97">{footer}</text>
    <text x="{right}" y="556" font-size="24" fill="#5c646e" text-anchor="end">bitgoose.com</text>
  </g>
</svg>"##,
        W = W,
        H = H,
        accent = accent,
        mark = mark(80.0, 58.0, 52.0, accent),
        section = esc(&card.section.to_uppercase()),
        label_x = W - 160,
        size = size,
        tspans = tspans,
        footer = esc(&footer),
        right = W - 80,
    )
}

/// The square card, for clients that crop to one.
///
/// Not the wide card with its sides trimmed. The frame is different and so is
/// the *viewing size*: a WeChat link preview is a thumbnail about a hundred
/// pixels across, sitting beside the headline and standfirst which WeChat
/// renders as text on its own. Setting the headline again inside a hundred
/// pixels produces grey texture, so the composition is centred on the mark —
/// the one element that still reads at that size, and the reason a Reuters
/// link in the same chat window looks like Reuters.
///
/// The headline is still set below it, for the clients that show this bigger.
fn square(card: &Card<'_>) -> String {
    let accent = accent(card.beat);
    // Two lines, generously sized. The headline is supporting matter here, not
    // the subject, and a four-line block would fight the mark.
    let (size, per_line) = match card.headline.chars().count() {
        0..=60 => (40.0_f32, 30),
        _ => (34.0, 36),
    };
    let lines = wrap(card.headline, per_line, 2);
    let mut tspans = String::new();
    for (i, line) in lines.iter().enumerate() {
        tspans.push_str(&format!(
            r#"<tspan x="{}" dy="{}">{}</tspan>"#,
            SQ / 2,
            if i == 0 { 0.0 } else { size * 1.25 },
            esc(line)
        ));
    }
    let n = card.sources.max(1);
    let mut footer = format!("{n} source{}", if n == 1 { "" } else { "s" });
    if card.has_analysis {
        footer.push_str("  ·  with analysis");
    }

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{SQ}" height="{SQ}" viewBox="0 0 {SQ} {SQ}">
  <rect width="{SQ}" height="{SQ}" fill="#0b0d10"/>
  <rect x="0" y="0" width="{SQ}" height="10" fill="{accent}"/>
  <g font-family="Ubuntu, DejaVu Sans, Liberation Sans, Arial, sans-serif" text-anchor="middle">
    {mark}
    <text x="{mid}" y="450" font-size="56" font-weight="700" fill="{accent}"
          letter-spacing="8">BITGOOSE</text>
    <text x="{mid}" y="492" font-size="24" fill="#838c97" letter-spacing="6">{section}</text>
    <text x="{mid}" y="580" font-size="{size}" font-weight="700" fill="#edeae3">{tspans}</text>
    <text x="{mid}" y="742" font-size="24" fill="#5c646e">{footer}</text>
  </g>
</svg>"##,
        SQ = SQ,
        mid = SQ / 2,
        accent = accent,
        // Centred and large: at thumbnail size this is the entire card.
        mark = mark((SQ as f32 - 230.0) / 2.0, 150.0, 230.0, accent),
        section = esc(&card.section.to_uppercase()),
        size = size,
        tspans = tspans,
        footer = esc(&footer),
    )
}

/// Rasterise a card to PNG. `None` when no font is available.
pub fn png(card: &Card<'_>) -> Option<Vec<u8>> {
    png_shaped(card, Shape::Wide)
}

/// Rasterise at a given shape. `None` when no font is available.
pub fn png_shaped(card: &Card<'_>, shape: Shape) -> Option<Vec<u8>> {
    let db = fonts()?;
    let mut opts = resvg::usvg::Options {
        fontdb: std::sync::Arc::new(db.clone()),
        ..Default::default()
    };
    opts.font_family = "Ubuntu".to_string();

    let (w, h) = shape.size();
    let tree = resvg::usvg::Tree::from_str(&shaped(card, shape), &opts).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(headline: &str) -> Card<'_> {
        Card {
            headline,
            beat: "ai",
            section: "Models",
            sources: 3,
            has_analysis: true,
        }
    }

    #[test]
    fn a_headline_with_markup_characters_cannot_break_the_document() {
        // An unescaped `&` yields invalid XML, and the card silently becomes
        // the static fallback — the failure is a missing feature, not an error.
        let c = card("Nvidia & AMD <clash> over \"agents\"");
        let out = svg(&c);
        assert!(out.contains("&amp;"), "ampersand not escaped");
        assert!(
            !out.contains("<clash>"),
            "raw tag survived into the document"
        );
        assert!(resvg::usvg::Tree::from_str(&out, &Default::default()).is_ok());
    }

    #[test]
    fn every_line_sets_its_own_x() {
        // A tspan without x continues the previous line instead of starting
        // beneath it, which stacks the whole headline into one long row.
        let c = card("A fairly long headline that will certainly need to wrap onto several lines");
        let out = svg(&c);
        let spans = out.matches("<tspan x=\"80\"").count();
        assert!(spans >= 2, "expected wrapped lines, got {spans}");
    }

    #[test]
    fn a_very_long_headline_is_truncated_and_says_so() {
        let long = "word ".repeat(80);
        let lines = wrap(&long, 30, 3);
        assert_eq!(lines.len(), 3);
        assert!(lines[2].ends_with('…'), "truncation should be visible");
    }

    #[test]
    fn a_short_headline_is_not_marked_truncated() {
        let lines = wrap("Bitcoin falls", 30, 3);
        assert_eq!(lines, vec!["Bitcoin falls"]);
    }

    #[test]
    fn a_single_source_is_not_pluralised() {
        // "1 sources" was on the first card rendered, in 24px, at the bottom of
        // every share of a single-source story.
        let c = Card {
            headline: "One outlet only",
            beat: "ai",
            section: "Policy",
            sources: 1,
            has_analysis: false,
        };
        let out = svg(&c);
        assert!(out.contains("1 source"), "missing the count");
        assert!(!out.contains("1 sources"), "pluralised a single source");
    }

    #[test]
    fn several_sources_are_pluralised() {
        let c = Card {
            headline: "Widely reported",
            beat: "crypto",
            section: "Markets",
            sources: 4,
            has_analysis: false,
        };
        assert!(svg(&c).contains("4 sources"));
    }

    #[test]
    fn each_desk_gets_its_own_accent() {
        // The colour is the only thing distinguishing four otherwise identical
        // layouts in a timeline.
        let mut seen: Vec<&str> = ["ai", "crypto", "markets", "tech"]
            .iter()
            .map(|b| accent(b))
            .collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 4, "two desks share an accent");
    }
}

#[cfg(test)]
mod render_preview {
    use super::*;
    /// Writes both cards to $BG_CARD_OUT so they can be looked at. A card that
    /// compiles is not a card that reads.
    #[test]
    fn emit() {
        let Ok(dir) = std::env::var("BG_CARD_OUT") else {
            return;
        };
        let c = Card {
            headline: "China says Long March 7A rocket failed after flight anomaly",
            beat: "ai",
            section: "Space",
            sources: 6,
            has_analysis: true,
        };
        for (shape, name) in [(Shape::Wide, "wide"), (Shape::Square, "square")] {
            if let Some(b) = png_shaped(&c, shape) {
                std::fs::write(format!("{dir}/card-{name}.png"), b).unwrap();
            }
        }
    }
}
