<div align="center">

# 🪿 BitGoose

**The AI-era newsroom for frontier technology.**

Ten AI agents run a newsroom end to end — polling sources, clustering events,
extracting claims, verifying them across outlets, and publishing. No humans in
the publishing path. Every claim shows its sources and a confidence score.

Two desks: **AI** — models, research, compute, safety and policy — and
**crypto**, where the project started.

[Architecture](#architecture) · [Quickstart](#quickstart) · [The Flock](#the-flock) ·
[API](#api--mcp) · [Editorial policy](#editorial-policy)

</div>

---

## Why this exists

Decrypt and CoinDesk are human newsrooms with a CMS bolted on. They publish
*articles* — opaque blobs of prose with a byline. Neither exposes why a claim is
believed, how many independent outlets confirmed it, or what changed after
publication. And neither is machine-readable: an AI agent that wants crypto news
has to scrape HTML and hope.

BitGoose inverts the stack. **The atomic unit is not the article — it's the
claim.**

```
raw items  ─▶  stories (events)  ─▶  claims (+ provenance, confidence)  ─▶  articles
```

Five outlets covering one hack produce five raw items and exactly **one** story.
That story decomposes into individual checkable claims, each tied to the specific
sources that support it — and to the ones that contradict it. An article is a
*rendering* of that claim set, not the source of truth.

Three things fall out of the inversion:

| | |
|---|---|
| **Provenance is the product** | Every figure carries its source count, corroboration state and confidence. Click any sentence, see what backs it. |
| **The newsroom is glass** | [`/flock`](#the-flock) shows all ten agents live — what each is doing, which model, how many tokens, what it cost, how often it fails. Published, not marketed. |
| **It's infrastructure, not just a site** | The claim graph ships as a REST API *and an MCP server*, so other AI agents consume BitGoose as a tool rather than parsing pages. |

---

## Architecture

Rust end to end. One binary serves the site, the API and the MCP endpoint.

```
                       ┌───────────────────────────────────────────┐
   9 RSS sources ──▶   │  Scout ─▶ Gosling ─▶ Curator              │
   CoinGecko/Coinbase  │                          │                │
                       │             ┌────────────┴───────────┐    │
                       │         [Desk]                    [Wire]  │
                       │   Scribe ─▶ Sentinel ─▶ Quant        │    │
                       │      ─▶ Copydesk ─▶ Gander        Herald  │
                       │                          │           │    │
                       │                      published ◀─────┘    │
                       │                          │                │
                       │                       Ombuds ─▶ corrections│
                       └───────────────────────────────────────────┘
                                        │
                            PostgreSQL 17 + pgvector
                                        │
                        ┌───────────────┼───────────────┐
                     Leptos SSR      /v1 REST         /mcp
```

| Crate | Role |
|---|---|
| `bg-core` | Domain model + the editorial policy engine. **WASM-safe** — shared with the hydrated client. |
| `bg-db` | PostgreSQL 17 + pgvector. 17 tables, sqlx migrations, repository layer. |
| `bg-ingest` | Polite feed polling: conditional GET, robots.txt, rate limits, URL canonicalization, SimHash dedupe. |
| `bg-llm` | Multi-provider LLM: Anthropic, any OpenAI-compatible endpoint (incl. Ollama), and a deterministic offline stub. Per-role model routing, failover, cost ledger. |
| `bg-agents` | The Flock — ten agents and the pipeline that runs them. |
| `bg-api` | Public REST + MCP server. |
| `bg-web` | Leptos SSR + hydration. The site. |
| `bg-cli` | `bg` — migrate, seed, ingest, run, worker, doctor. |

**The constraint that governs the build:** `bg-web`'s lib compiles to
`wasm32-unknown-unknown` for hydration, so `bg-core` must be WASM-safe — no
tokio, no sqlx, no reqwest. Everything native is pulled in only under
`feature = "ssr"`.

---

## Quickstart

Requires Rust 1.90+, Docker, and [`cargo-leptos`](https://github.com/leptos-rs/cargo-leptos).

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked cargo-leptos

git clone https://github.com/finalverse/bitgoose.git
cd bitgoose
cp .env.example .env          # works as-is; no API key needed

docker compose up -d          # Postgres 17 + pgvector
cargo run -p bg-cli -- migrate
cargo run -p bg-cli -- seed   # 9 sources, 12 assets, 15 entities, 10 agents
cargo run -p bg-cli -- doctor # verify everything is wired up

cargo run -p bg-cli -- run    # one full newsroom pass against live feeds
cargo leptos watch            # http://127.0.0.1:3000
```

**It runs with no API key.** The default provider is a deterministic offline
stub that synthesizes schema-conforming output — so the entire pipeline,
including clustering, the policy engine, the database writes and the rendering,
is exercised end to end at zero cost. Set `ANTHROPIC_API_KEY` and
`BG_LLM_PROVIDER=anthropic` in `.env` for real output.

### `bg` commands

```
bg migrate            apply database migrations
bg seed               sources, assets, entities, agent roster
bg doctor             db + pgvector + source reachability + LLM status
bg ingest             poll every due source once
bg run                one full newsroom pass
bg worker --interval  run the pipeline on a loop
bg stats              24h newsroom statistics
bg violations         recent policy blocks
```

---

## The Flock

Ten agents. Every stage writes a row to `agent_runs` — LLM-backed or not,
success or failure — and that table is public at `/flock`.

| Agent | Beat | Tier |
|---|---|---|
| **Scout** | Watches every source, around the clock | none (deterministic) |
| **Gosling** | First read on everything that lands | fast |
| **Curator** | Decides what is one story and what is five | fast |
| **Scribe** | Extracts the claims and writes the draft | mid |
| **Sentinel** | Checks every claim against every source | top |
| **Quant** | Puts the numbers in context | mid |
| **Copydesk** | Headlines, deks and house style | fast |
| **Gander** | Editor-in-chief. Publishes, holds, or kills | top |
| **Herald** | Gets it to the Wire, the inbox and the feed | fast |
| **Ombuds** | Re-reads what we published and corrects it | mid |

Agents never name a model — they request a capability tier, and `bg-llm`
resolves it per provider. Switching the whole newsroom from Anthropic to a local
Ollama is one environment variable.

### Two design decisions worth calling out

**Confidence is a fact about the evidence, not a model's opinion.** Sentinel's
verdict passes through a deterministic floor: a claim with one independent
source can never be rated `corroborated`, no matter how confident the model
sounded. The model can only ever *lower* a rating. That single rule is what
stops the site looking authoritative about things nobody actually confirmed.

**Clustering splits when uncertain.** Merging two unrelated events is far worse
than splitting one — everything downstream would treat them as corroborating
each other. So the cascade is: cheap lexical matching (SimHash + trigram)
settles the clear cases for free, an LLM adjudicates only the ambiguous middle
band, and anything unresolved becomes a new story.

---

## Editorial policy

BitGoose reads other people's journalism. That is only defensible if the boundary
between *reading* and *reproducing* is mechanical rather than aspirational —
prompt instructions are not a control.

So the rules live in `bg-core::policy`, run on the path to `status = published`,
and a draft that fails **cannot** be published:

- Quotes capped at **25 words**, attributed, with a link out.
- No run longer than **28 words** may match any source — this catches lifted
  wording even when it was never marked as a quote.
- Every claim carries at least one source, or it does not ship.
- A `refuted` claim can never appear in published prose.
- An original story needs at least **2 independent sources**.
- The AI-authorship disclosure must be present.

Source text is ingested to a private column for analysis and **never served**.
Every refused publish is written to `policy_violations` — a block that is logged
and forgotten is a block that recurs.

The database enforces the quote cap independently, via a `CHECK` constraint, so
a bug in the policy path still cannot store an over-long excerpt.

---

## API & MCP

Public, unauthenticated, CORS-open.

```
GET  /v1/stories?kind=desk&limit=20
GET  /v1/stories/{slug}      # full claim ledger + how it was produced
GET  /v1/wire
GET  /v1/claims/{id}         # one claim, every source
GET  /v1/prices
GET  /v1/assets/{ticker}
GET  /v1/flock               # live agent cost and error rate
GET  /v1/standards           # policy + enforcement record
POST /mcp                    # JSON-RPC 2.0
```

The MCP server exposes five tools: `search_stories`, `get_story`,
`verify_claim`, `get_prices`, `newsroom_status`.

`verify_claim` is the interesting one. Instead of a headline, it returns matching
claims with their verification state, confidence and the independent outlets
behind each — so an agent can tell the difference between something two
newsrooms confirmed and something one account posted.

```bash
curl -s localhost:3000/mcp -H 'content-type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"tools/call",
  "params":{"name":"verify_claim","arguments":{"query":"exchange froze attacker funds"}}}'
```

**Terms.** Claims and metadata are freely reusable with attribution. Source text
is never redistributed through this API, because it is not ours to redistribute.

---

## Sources

Thirty-two, weighted by a trust score that reflects *editorial process* — named
reporters, a corrections policy, original reporting vs reprints — not whether we
like the coverage. Scores are visible on `/standards` precisely because they are
contestable.

**AI** — OpenAI · Google DeepMind · Hugging Face · TechCrunch · Ars Technica ·
MIT Technology Review · The Verge · Simon Willison · Import AI ·
arXiv cs.AI · arXiv cs.LG · r/MachineLearning · r/LocalLLaMA

**Crypto** — CoinDesk · The Block · Decrypt · DL News · Blockworks ·
The Defiant · Bitcoin Magazine · Cointelegraph · CryptoSlate

**Mainstream finance** — Bloomberg · Financial Times · CNBC · MarketWatch ·
Yahoo Finance. Their feeds are mostly equities and rates, so each item passes a
crypto/AI relevance gate before it is stored rather than being taken wholesale.

**Video** — Coin Bureau · Bankless · Milk Road · Unchained · Crypto Banter,
embedded through YouTube's own player so the creator keeps control.

Market data from CoinGecko, with Coinbase as fallback.

Not every source kind is a news article, and the interface says so: a preprint
has no editor and no peer review, and a forum thread is an argument rather than
a report. Both are tagged as such and neither counts as corroboration for a
claim.

**Why there is no X/Twitter.** Its API starts at $200/month, and the free route
is third-party scrapers that work intermittently and operate against X's terms.
A newsroom that publishes a [`/bot`](https://bitgoose.com/bot) page promising to
honour robots.txt should not run on a scraping proxy.

Four candidates were tested and rejected: Anthropic publishes no RSS, Hugging
Face's papers feed requires auth, VentureBeat's AI feed stopped updating, and
Reuters' public RSS endpoints 404.

---

## Contributing

```bash
cargo test --workspace                                   # unit + integration
cargo build -p bg-core --target wasm32-unknown-unknown   # WASM boundary check
cargo clippy --workspace --all-targets
```

Integration tests need Postgres running (`docker compose up -d`); they create
and drop their own scratch databases and skip cleanly if nothing is reachable.

**Never commit a `.env`.** A pre-commit hook scans staged changes for
credentials — install it with:

```bash
git config core.hooksPath .githooks
```

---

## License

MIT. See [LICENSE](LICENSE).
