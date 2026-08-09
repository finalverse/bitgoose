# Multi-lingual BitGoose — a plan

Status: **plan, not yet built.** The groundwork (language tag normalisation) is
in. Everything below is a decision record so the build is a series of small
steps rather than one large guess.

## Where we start

The corpus is 100% English: 1,396 items, spelled `en-us`, `en` and `en-US` —
three tags for one language, which is why `text::normalize_lang` now exists. A
per-language surface built on the raw value would have missed most of its own
rows.

`lang` was stored and read by nothing. That is the first thing to change, and
it is cheap.

## The decision that shapes everything else

There are two products here and they are not the same:

**(a) Translate BitGoose into other languages.** One newsroom, many renderings.
The Chinese reader gets our English-sourced analysis in Chinese.

**(b) Report in other languages from sources in those languages.** A Chinese
desk reading Chinese sources, whose stories may never exist in English.

(a) is a translation feature. (b) is a second newsroom.

**Recommendation: (b), starting narrow.** (a) is cheaper and worse — it makes
BitGoose a machine-translated mirror of an anglophone view, competing with
Google Translate on someone else's reporting. (b) is the only version that
produces something a reader cannot get elsewhere: Chinese and Japanese crypto
and AI coverage that English-language outlets do not carry, run through the same
claim graph and the same corroboration standard.

The Skein's whole argument is that a claim shows its sources. That argument
travels; a translation layer does not.

## Why this is now feasible

Until v0.12.0 a source had to publish RSS. Most non-English outlets that matter
here do not, or publish a truncated one. `bg-ingest::crawl` reads index pages
directly, so the source roster is no longer limited to publishers who chose to
emit XML.

## Order of work

**1. Make `lang` real** (small, no model cost)
- Store the normalised tag — done at ingest
- Backfill the existing corpus
- Filter every surface by language; default to the reader's `Accept-Language`,
  overridable and remembered
- `hreflang` on every page, so search engines see one story in N languages
  rather than N duplicate pages

**2. Language-aware relevance** (small)
`bg-ingest::relevance` routes by English keywords. A Chinese headline about
比特币 matches nothing and is dropped before it reaches triage. The term tables
need a per-language set; this is a data problem, not a model one.

**3. Trend detection per language** (small)
`bg-core::trends` extracts capitalised runs, which is an assumption about
*English orthography*. Chinese and Japanese have no capitalisation and no spaces
— the extractor returns nothing. Options, cheapest first:
- CJK: n-gram frequency over character bigrams/trigrams, scored the same way
  (independent-source convergence). No model, no dictionary.
- A per-language stopword list, as English has.
Convergence scoring itself is language-neutral and does not change.

**4. Sources** (data)
Candidate first languages, chosen for depth of coverage in this beat:
- **Chinese** — 链闻/ChainNews, 巴比特, 36氪 (tech). Check robots.txt first;
  several are crawl-friendly, some are not.
- **Japanese** — CoinPost, ITmedia.
- **Korean** — 코인데스크코리아 (CoinDesk Korea is a separate newsroom).
Each needs the robots check and a trust score before it goes in.

**5. The Flock, per language** (model cost — the expensive step)
Gosling, Herald and the Skein all write. Their prompts are English and their
output must match the source language, not the prompt language.
- The house style prompt needs a language directive per run
- The grounding gate and verbatim quote check are language-neutral already —
  `contains_verbatim` normalises typography, not script, so it works on CJK
  unchanged
- **Cost**: this multiplies token spend by the number of active languages. On
  200,000 tokens/day it is not affordable for even one more language without a
  paid tier. This step is gated on that, and should be stated plainly rather
  than discovered halfway.

**6. Cross-language corroboration** (the actually interesting part)
Two outlets in two languages reporting the same event is *stronger* evidence
than two in one — they share no editor, no wire, no press release cycle. The
Curator clusters on lexical similarity, which cannot see across scripts.
Doing this properly needs embeddings (`BG_EMBED_PROVIDER` exists and is off).
This is the feature that would make BitGoose genuinely different, and it is
last because it depends on all of the above.

## What not to do

- **Do not machine-translate our own analysis and present it as native.** A
  translated Skein forecast reads as authored in the target language and is not.
  If a translation is shown, it says so.
- **Do not mix languages on one surface.** A reader wants a newsroom, not a
  multilingual pile.
- **Do not let a language desk lower the corroboration bar** because it has
  fewer sources. Three Chinese outlets is three outlets; two is single-sourced
  in any language.

## Open questions for the operator

1. Which language first? Recommendation: **Chinese** — largest non-English
   crypto readership and a genuine information gap.
2. Paid inference tier? Step 5 is blocked without it.
3. Do we want (a) as an interim — English stories with a translated summary —
   or wait for (b)? I would wait; a mirror is not worth the tokens.
