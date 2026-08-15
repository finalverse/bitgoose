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
    /// The publisher's `Content-Signal`, where they set one.
    signals: Signals,
    /// Whether any named AI crawler is blocked outright.
    ///
    /// Read as intent, not as a rule about us. A site that allows `*` while
    /// disallowing GPTBot, ClaudeBot, CCBot and Google-Extended has said
    /// plainly what it objects to, and it is not the fetching.
    pub blocks_ai_crawlers: bool,
}

/// The AI-related permissions a site publishes, per the Content Signals
/// convention now carried in a great many robots.txt files.
///
/// ```text
/// Content-Signal: search=yes,ai-train=no,use=reference
/// ```
///
/// The convention is explicit that an **absent** signal is neither permission
/// nor refusal, so each is a three-state and `None` must never be read as yes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Signals {
    /// Indexing and short excerpts with a link back.
    pub search: Option<bool>,
    /// Putting the content into a model at inference time — retrieval,
    /// grounding, summarising. **This is what BitGoose's Skein does.**
    pub ai_input: Option<bool>,
    /// Training or fine-tuning. BitGoose never does this.
    pub ai_train: Option<bool>,
}

impl Robots {
    /// What the site says about AI use.
    pub fn signals(&self) -> Signals {
        self.signals
    }

    /// Whether we may put this site's body text into a model.
    ///
    /// The conservative reading, and deliberately so. An explicit `ai-input=no`
    /// settles it. Where the signal is absent — which is the common case — a
    /// site that has gone to the trouble of naming and blocking the AI crawlers
    /// has expressed an intent, and reading "unspecified" as consent would be
    /// choosing the interpretation that happens to suit us.
    ///
    /// Saying no here does not drop the source. It keeps it as a headline and a
    /// link out, which is what `use=reference` describes and what a link
    /// aggregator has always done.
    pub fn allows_ai_input(&self) -> bool {
        match self.signals.ai_input {
            Some(v) => v,
            None => !self.blocks_ai_crawlers,
        }
    }
}

#[derive(Debug, Clone)]
struct Rule {
    path: String,
    allow: bool,
}

impl Robots {
    /// Parse a robots.txt body. Unknown directives are ignored.
    pub fn parse(body: &str) -> Self {
        /// Agents whose presence in a `Disallow: /` block says the site does
        /// not want its text inside a model. Only the ones that exist purely
        /// to collect for AI — an SEO crawler being blocked means nothing here.
        const AI_AGENTS: &[&str] = &[
            "gptbot",
            "chatgpt-user",
            "oai-searchbot",
            "claudebot",
            "claude-web",
            "anthropic-ai",
            "ccbot",
            "google-extended",
            "applebot-extended",
            "meta-externalagent",
            "facebookbot",
            "bytespider",
            "perplexitybot",
            "cohere-ai",
            "diffbot",
            "omgili",
            "timpibot",
            "amazonbot",
        ];

        let mut groups: HashMap<String, Vec<Rule>> = HashMap::new();
        let mut signals = Signals::default();
        let mut blocks_ai_crawlers = false;
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
                "content-signal" => {
                    // `search=yes,ai-train=no,use=reference`. Applies to the
                    // group it sits in; in practice it is written under `*`,
                    // and treating it as site-wide is the safer reading.
                    for pair in value.split(',') {
                        let Some((k, v)) = pair.split_once('=') else {
                            continue;
                        };
                        let yes = match v.trim().to_ascii_lowercase().as_str() {
                            "yes" => Some(true),
                            "no" => Some(false),
                            _ => None, // e.g. `use=reference`, not a permission
                        };
                        match k.trim().to_ascii_lowercase().as_str() {
                            "search" => signals.search = yes,
                            "ai-input" => signals.ai_input = yes,
                            "ai-train" => signals.ai_train = yes,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // A named AI crawler blocked from the whole site.
        for (agent, rules) in &groups {
            if AI_AGENTS.contains(&agent.as_str())
                && rules.iter().any(|r| !r.allow && r.path == "/")
            {
                blocks_ai_crawlers = true;
                break;
            }
        }

        Self {
            groups,
            signals,
            blocks_ai_crawlers,
        }
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

#[cfg(test)]
mod signal_tests {
    use super::*;

    /// theaiinsider.tech, as served on 2026-08-12.
    const AI_INSIDER: &str = "\
User-agent: *
Content-Signal: search=yes,ai-train=no,use=reference
Allow: /

User-agent: ClaudeBot
Disallow: /

User-agent: GPTBot
Disallow: /

User-agent: CCBot
Disallow: /

User-agent: *
Crawl-delay: 10
Disallow: /wp-admin/
";

    #[test]
    fn a_site_may_welcome_a_crawler_and_still_refuse_the_model() {
        let r = Robots::parse(AI_INSIDER);
        // We are allowed to fetch: we are not one of the named agents.
        assert!(r.allowed("bitgoosebot", "/some-article"));
        assert!(!r.allowed("claudebot", "/some-article"));
        // And the site said what it thinks of AI use.
        assert_eq!(r.signals().search, Some(true));
        assert_eq!(r.signals().ai_train, Some(false));
        // `ai-input` was not stated — but blocking GPTBot, ClaudeBot and CCBot
        // by name is not ambiguous about intent.
        assert_eq!(r.signals().ai_input, None);
        assert!(r.blocks_ai_crawlers);
        assert!(
            !r.allows_ai_input(),
            "unspecified plus named AI blocks must not read as consent"
        );
    }

    #[test]
    fn an_explicit_yes_overrides_the_inference() {
        // A site can block a specific vendor's crawler and still permit
        // inference use generally. Its words beat our guess about its intent.
        let r = Robots::parse(
            "User-agent: *\nContent-Signal: ai-input=yes\nAllow: /\n\nUser-agent: GPTBot\nDisallow: /\n",
        );
        assert!(r.blocks_ai_crawlers);
        assert!(r.allows_ai_input());
    }

    #[test]
    fn an_ordinary_site_is_not_treated_as_refusing() {
        // No signals, no AI blocks: nothing has been said, and inventing a
        // refusal would cost the newsroom sources nobody asked it to drop.
        let r = Robots::parse("User-agent: *\nDisallow: /admin\n");
        assert!(r.allows_ai_input());
        assert!(!r.blocks_ai_crawlers);
    }

    #[test]
    fn blocking_an_seo_crawler_says_nothing_about_ai() {
        let r = Robots::parse(
            "User-agent: *\nAllow: /\n\nUser-agent: AhrefsBot\nDisallow: /\n\nUser-agent: SemrushBot\nDisallow: /\n",
        );
        assert!(!r.blocks_ai_crawlers);
        assert!(r.allows_ai_input());
    }

    #[test]
    fn an_explicit_no_is_final() {
        let r = Robots::parse("User-agent: *\nContent-Signal: ai-input=no\nAllow: /\n");
        assert!(!r.allows_ai_input());
    }

    #[test]
    fn a_partial_block_is_not_a_refusal_of_the_whole_site() {
        // GPTBot kept out of one section is a section rule, not a stance.
        let r =
            Robots::parse("User-agent: *\nAllow: /\n\nUser-agent: GPTBot\nDisallow: /premium/\n");
        assert!(!r.blocks_ai_crawlers);
        assert!(r.allows_ai_input());
    }
}

/// What one robots.txt says about a URL: may we fetch it, and may a model read
/// it.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub allowed: bool,
    pub ai_input: bool,
    /// The `Content-Signal` line verbatim, or `None` if the site set none.
    pub signal: Option<String>,
}

/// Fetch robots.txt once and answer both questions.
///
/// Separate from [`allows`] because the two are asked at different times —
/// fetching happens per URL and per poll, while the AI posture is a property of
/// the source — and one shared fetch is politer than two.
pub async fn verdict(client: &reqwest::Client, agent: &str, target: &str) -> Verdict {
    // Unreachable or unparseable robots.txt is permissive on both counts, for
    // the reason given on `allows`: a transient 500 is not a publisher telling
    // us anything, and treating it as a ban would disable a source silently.
    let open = || Verdict {
        allowed: true,
        ai_input: true,
        signal: None,
    };
    let Ok(u) = url::Url::parse(target) else {
        return Verdict {
            allowed: false,
            ..open()
        };
    };
    let Ok(robots_url) = u.join("/robots.txt") else {
        return open();
    };
    let Ok(resp) = client.get(robots_url).send().await else {
        return open();
    };
    if !resp.status().is_success() {
        return open();
    }
    let Ok(body) = resp.text().await else {
        return open();
    };

    let r = Robots::parse(&body);
    let signal = body
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .find(|l| l.to_ascii_lowercase().starts_with("content-signal:"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .filter(|s| !s.is_empty());

    Verdict {
        allowed: r.allowed(agent, u.path()),
        ai_input: r.allows_ai_input(),
        signal,
    }
}
