//! A small, strict robots.txt checker.
//!
//! Deliberately conservative in the direction that costs us stories rather than
//! goodwill: an unparseable or ambiguous rule is treated as "disallowed". We
//! are a bot reading other people's servers at scale, and being wrong in the
//! other direction is how a crawler gets blocked at the CDN and stays blocked.
//!
//! Not a full RFC 9309 implementation — no crawl-delay scheduling, no wildcard
//! `$` anchoring. It covers what publishers actually write.

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Robots {
    /// Disallow prefixes per user-agent token (lowercased).
    groups: HashMap<String, Vec<Rule>>,
}

#[derive(Debug, Clone)]
struct Rule {
    path: String,
    allow: bool,
}

impl Robots {
    /// Parse a robots.txt body. Unknown directives are ignored.
    pub fn parse(body: &str) -> Self {
        let mut groups: HashMap<String, Vec<Rule>> = HashMap::new();
        // Consecutive `User-agent:` lines share one rule block, per the spec.
        let mut current: Vec<String> = Vec::new();
        let mut expecting_agents = true;

        for line in body.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();

            match key.as_str() {
                "user-agent" => {
                    if !expecting_agents {
                        current.clear();
                        expecting_agents = true;
                    }
                    current.push(value.to_ascii_lowercase());
                }
                "disallow" | "allow" => {
                    expecting_agents = false;
                    let allow = key == "allow";
                    // `Disallow:` with an empty value means "allow everything".
                    if value.is_empty() && !allow {
                        for a in &current {
                            groups.entry(a.clone()).or_default();
                        }
                        continue;
                    }
                    for a in &current {
                        groups.entry(a.clone()).or_default().push(Rule {
                            path: value.to_string(),
                            allow,
                        });
                    }
                }
                _ => {}
            }
        }
        Self { groups }
    }

    /// Whether `path` may be fetched by `agent`.
    ///
    /// Longest-match wins, and `Allow` beats `Disallow` at equal length — the
    /// behaviour every major crawler implements, and what publishers assume
    /// when they write `Disallow: /` followed by `Allow: /feed`.
    pub fn allowed(&self, agent: &str, path: &str) -> bool {
        let agent = agent.to_ascii_lowercase();
        // Most specific group first: our own token, then `*`.
        let rules = self
            .groups
            .iter()
            .filter(|(k, _)| *k != "*" && agent.contains(k.as_str()))
            .map(|(_, v)| v)
            .next()
            .or_else(|| self.groups.get("*"));

        let Some(rules) = rules else { return true };

        let mut best: Option<&Rule> = None;
        for r in rules {
            if !path.starts_with(&r.path) {
                continue;
            }
            match best {
                None => best = Some(r),
                Some(b) if r.path.len() > b.path.len() => best = Some(r),
                // Equal specificity: Allow wins.
                Some(b) if r.path.len() == b.path.len() && r.allow && !b.allow => best = Some(r),
                _ => {}
            }
        }
        best.map(|r| r.allow).unwrap_or(true)
    }
}

/// Fetch and evaluate robots.txt for one URL.
///
/// A network failure yields `true`. That is the one place we are permissive:
/// treating a transient 500 on robots.txt as a site-wide ban would silently
/// disable a source and leave no obvious trace of why.
pub async fn allows(client: &reqwest::Client, agent: &str, target: &str) -> bool {
    let Ok(u) = url::Url::parse(target) else {
        return false;
    };
    let Ok(robots_url) = u.join("/robots.txt") else {
        return true;
    };

    let Ok(resp) = client.get(robots_url).send().await else {
        return true;
    };
    if !resp.status().is_success() {
        // 404 means no restrictions; anything else we also treat as open,
        // having no rules to apply.
        return true;
    }
    let Ok(body) = resp.text().await else {
        return true;
    };
    Robots::parse(&body).allowed(agent, u.path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_disallow_blocks_matching_prefixes() {
        let r = Robots::parse("User-agent: *\nDisallow: /private\nDisallow: /admin\n");
        assert!(!r.allowed("BitGooseBot", "/private/x"));
        assert!(!r.allowed("BitGooseBot", "/admin"));
        assert!(r.allowed("BitGooseBot", "/feed"));
    }

    #[test]
    fn empty_disallow_means_everything_is_allowed() {
        let r = Robots::parse("User-agent: *\nDisallow:\n");
        assert!(r.allowed("BitGooseBot", "/anything"));
    }

    #[test]
    fn allow_overrides_a_broader_disallow() {
        let r = Robots::parse("User-agent: *\nDisallow: /\nAllow: /feed\n");
        assert!(r.allowed("BitGooseBot", "/feed"));
        assert!(!r.allowed("BitGooseBot", "/article/1"));
    }

    #[test]
    fn a_named_group_takes_precedence_over_the_wildcard() {
        let r = Robots::parse("User-agent: *\nDisallow:\n\nUser-agent: bitgoosebot\nDisallow: /\n");
        assert!(
            !r.allowed("BitGooseBot/0.1", "/feed"),
            "our own rule must win"
        );
        assert!(r.allowed("SomeOtherBot", "/feed"));
    }

    #[test]
    fn consecutive_user_agent_lines_share_one_block() {
        let r = Robots::parse("User-agent: a\nUser-agent: b\nDisallow: /x\n");
        assert!(!r.allowed("a", "/x"));
        assert!(!r.allowed("b", "/x"));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let r = Robots::parse("# hello\n\nUser-agent: *   # everyone\nDisallow: /p  # private\n");
        assert!(!r.allowed("x", "/p"));
        assert!(r.allowed("x", "/q"));
    }

    #[test]
    fn an_empty_file_allows_everything() {
        assert!(Robots::parse("").allowed("x", "/anything"));
    }
}

#[cfg(test)]
mod reddit_regression {
    use super::*;

    /// Reddit disallows everything, for everyone, including their `.rss`
    /// endpoints. Our stored `robots_ok` said otherwise, so this pins the
    /// parser against the real file rather than against a paraphrase of it.
    #[test]
    fn a_blanket_disallow_covers_the_feed_too() {
        let body = "# Welcome to Reddit's robots.txt\n\
                    # Reddit believes in an open internet, but not the misuse.\n\
                    # policy: https://support.reddithelp.com/hc/en-us\n\
                    \n\
                    User-agent: *\n\
                    Disallow: /\n";
        let r = Robots::parse(body);
        assert!(!r.allowed("BitGooseBot", "/r/LocalLLaMA/.rss"));
        // Also under the browser product token the fetcher actually sends.
        assert!(!r.allowed(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            "/r/LocalLLaMA/comments/abc/title"
        ));
    }
}
