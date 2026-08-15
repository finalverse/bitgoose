# CCC in BitGoose

**Status.** The mandate layer is built and running (v0.16.0). Everything below
the first section is a design, not shipped code. **CCC is currently a test
network token with no monetary value**, so nothing here moves money and nothing
here should be read as a claim that it does.

---

## The one-line version

BitGoose is eleven AI agents that spend money on inference and publish what they
find. CreditChain's core primitive is a *policy-bound agent wallet* — a budget,
a time window, an allowlist, so "machines transact under rules humans can
verify". Those are the same object. Denominating the Flock in CCC turns
BitGoose's central claim, the glass newsroom, from something you take on trust
into something you can check.

That is the whole reason to put a token here. Not to charge readers, not to mint
something to sell.

## Why not the obvious things

Worth stating what was rejected, because these are what a crypto news site is
normally tempted into:

- **Token-gated articles.** News that can only be read by holders is not news,
  it is a subscription with extra steps and worse reach. Reading BitGoose stays
  free and unauthenticated, permanently.
- **Read-to-earn.** Paying people to open pages buys bots. The metric it
  optimises is the one metric a newsroom must not optimise.
- **A separate BITGOOSE token.** The user asked for CCC and CCC is right:
  BitGoose does not need a currency, it needs a *unit of account for machine
  spending*, which is what CCC already is.
- **Selling the archive for training.** Nine of thirty-five sources decline
  model input. Reselling what they let us index would be exactly the behaviour
  they blocked GPTBot to prevent.

## 1. Agents spend under mandates — *built*

Each of the eleven agents holds a mandate: a daily CCC ceiling, a task
allowlist, and a maximum model tier. It is checked before any model call and
settled from the tokens actually returned.

```
MANDATE   0.0164 / 0.1000 CCC     ← on every tile at /flock
```

Three things this already does, before any chain is involved:

- **Contains a fault to one agent.** Previously one global budget meant a loop
  in the Skein exhausted the day for everyone, and the other ten failed with no
  indication why. Verified: with the budget set to zero, 49 stages refused, each
  recorded against its own agent with a reason.
- **Refuses work outside the role.** The Skein's mandate covers `skein.` and
  nothing else, so a stage wired to the wrong agent is declined rather than
  paid for.
- **Caps tier.** Tier spans roughly twentyfold in price; a triage agent that
  starts calling the top model is stopped long before a budget notices.

The figure on `/flock` is computed from `agent_runs`, the same ledger the worker
settles against — not from the worker's own belief about itself, which is
precisely the number a reader has no reason to trust.

**What settlement adds.** A mandate published on CreditChain is a commitment
made *in advance* and receipts that anyone can check against it. Today the page
says what we spent; then the chain says what we were authorised to spend, and
the two can be compared by someone who does not trust us at all. That is a
falsifiable accountability claim, which is the only kind worth making.

## 2. Machine readers pay, humans never do — *design*

BitGoose's real product is not the web page. It is the claim graph: every
assertion with its sources, corroboration count and confidence, over REST and
MCP. That is infrastructure for other AI agents, and an AI agent is a party that
*has* a wallet and a budget.

- **Humans: free, unauthenticated, forever.** No wallet, no login, no gate.
- **Machines: metered in CCC** above a generous free tier, paid from the caller's
  own policy-bound wallet.

This is the honest place to charge, because it is the only place where the party
being charged is a machine with a mandate of its own — agent-to-agent
settlement, which is what CreditChain exists for. It also prices the thing that
actually costs us money: a crawler hammering the API is a real inference and
bandwidth bill, and a reader is not.

## 3. Sources earn from corroboration — *design, and the interesting one*

Every aggregator has the same relationship with publishers: take the headline,
keep the attention, send back what traffic is left. BitGoose is currently no
different, and nine of its sources have already blocked the AI crawlers to say
what they think of it.

The claim graph makes a different arrangement computable. BitGoose knows, per
claim, **which outlets independently confirmed it**. So a share of API revenue
can flow to the outlets the graph rests on, weighted by corroborated citation —
not by clicks, not by volume.

What that pays for is worth being precise about. It pays an outlet more for
being *first and right and independently confirmed* than for being loud. A wire
story reprinted eight times earns once, split; a piece of original reporting that
three other newsrooms then confirm earns as the thing that anchored the claim.
No other aggregator can compute this, because no other aggregator tracks
provenance per claim — it is a direct consequence of the architecture rather
than a policy bolted on.

It also inverts the incentive on the ingestion side: BitGoose would be paying for
corroborated reporting, which is the same thing its ranking already rewards.

**Open question, honestly.** A publisher must be able to receive this without
adopting a wallet, or it reaches nobody. Escrowed-until-claimed is the obvious
answer and is not designed yet.

## 4. Reputation as a credit profile — *design*

CreditChain's AI layer does attestation and credit profiles. BitGoose already
computes a trust score per source and a corroboration record per claim. Those
are the same shape: a public, accumulating record of whether an outlet's
reporting held up.

Published as attestations, a newsroom's corroboration history becomes portable —
usable by anyone, not just by us. That is a genuinely new thing for the industry
and costs BitGoose nothing to emit, since it is already computed.

The obvious hazard is BitGoose becoming a self-appointed arbiter of which
outlets are credible. The mitigation has to be that the attestation records
*what happened* — this claim, these confirmations, this correction — and not a
verdict. A score anyone can recompute from the underlying record is a
measurement; a score only we can produce is a rating agency, and the industry
has enough of those.

## What must stay true

1. **Reading is free.** No wallet, no login, no gate. Non-negotiable.
2. **No token near the editorial decision.** Nothing about what is published,
   ranked, corrected or retracted may depend on payment, and no source's
   trust score may be purchasable. The policy engine enforces publication
   rules and must never learn about CCC.
3. **Nothing that looks like an investment.** Testnet CCC has no monetary value
   and the interface says so wherever a reader might assume otherwise. BitGoose
   is a newsroom; it does not offer yield, and it does not advise.
4. **Publishers' stated wishes bind regardless of payment.** A source that
   declines model input is not for sale.

## Sequence

| | |
|---|---|
| **Now** | Mandates enforced locally, denominated in CCC, visible on `/flock` |
| Next | Persist mandates and receipts so a restart cannot forget, as the pacer does |
| Next | Publish mandates and settle receipts on CreditChain testnet — read-only, no keys held by the newsroom process |
| Then | Meter the machine API in CCC, free tier for humans |
| Then | Corroboration payouts, once there is revenue to share and a way for a publisher to receive it |
| Later | Attestations for source reputation |

Steps three onward need a key and an explicit decision to move value. Neither is
something this codebase should take on its own.
