# BitGoose global editions

Status: **implemented.** BitGoose operates seven independent editorial editions:

| Edition | Path | Editorial language |
|---|---|---|
| English | `/` | `en` |
| 简体中文 | `/zh` | `zh` |
| 繁體中文 | `/zh-hant` | `zh-hant` |
| Français | `/fr` | `fr` |
| Español | `/es` | `es` |
| 日本語 | `/ja` | `ja` |
| 한국어 | `/ko` | `ko` |

These are native newsrooms, not translated mirrors. Each edition has its own
source queue, story selection, headlines, topic clusters and analysis. A story
published in one edition need not exist in another.

## Editorial architecture

- The source's configured language is authoritative at ingest. Language tags
  are normalized at the boundary, with Traditional Chinese kept separate from
  Simplified Chinese.
- Scheduling is language-aware and source-fair. English receives two slots per
  round; every other active edition receives one, preventing a high-volume
  English feed from starving a smaller newsroom.
- Story, topic, trend, entity and flyway queries are filtered by editorial
  language. Cross-language topic membership is rejected during migration.
- Trend detection handles Latin text, CJK text and Hangul without a model. A
  trend needs convergence from independent publishers; several feeds owned by
  one publisher count once.
- Every edition continuously tracks Bitcoin policy and institutional adoption,
  the frontier AI race, and AI chips/compute supply. Fast-moving clusters are
  promoted into additional language-local topics automatically.

## Sources

The global roster combines primary documents and local specialist reporting.
The Japanese edition includes CoinPost, ITmedia and あたらしい経済; Korean
includes AI Times, Blockmedia and TokenPost; Traditional Chinese includes
ABMedia, iThome and RTHK; Spanish includes Xataka, Genbeta, Hipertextual,
DiarioBitcoin, BeInCrypto Español and The Cryptonomist Español. Google News
queries provide discovery for AI, crypto and technology in each language, but
all links from the same Google News publisher are collapsed for independence
scoring.

## AI newsroom rules

`prompts/master-system.md` is the binding house prompt for every agent and
language. It requires source attribution, explicit separation of fact and
analysis, careful treatment of market claims, and original synthesis rather
than republishing copyrighted articles. Model output must be native to the
edition and may not be a mechanical translation of another edition.

## Operations

The worker refreshes all seven editions on every topic cycle. Ingest remains
conditional and polite (ETag/Last-Modified, robots checks and per-host pacing).
Adding a new language requires the same complete path: domain enum, normalized
ingest, local sources, relevance/trend coverage, scheduler slot, prompt output
contract, language-filtered queries, routes, navigation, SEO locale, durable
topics and migration coverage.
