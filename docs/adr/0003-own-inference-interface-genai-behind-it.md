# ADR 0003: Our own inference interface, with genai behind it

**Date:** 2026-10-03
**Status:** Accepted
**Owner:** Olli Varila
**Related:** [PRD](../prd.md) · [Spec](../spec/2026-10-03-inference-design.md)

## Context

The harness has to call Anthropic's Messages API, and there is no official
Rust SDK for it.

The genai crate offers a provider-neutral chat model with adapters for many
providers. That is the same thing our inference interface is. So the question
was not only whether to use it, but how far to let it in.

## Decision

### The harness speaks our own interface and data model

The boundary is one model call. The harness never sees provider data or genai
types.

- **Stability** — genai is pre-1.0 and its next release has breaking changes.
  If its types were ours, each of its breaks would break our tools. Behind
  our interface, a break touches one adapter.
- **Not the lowest common denominator** — a library for many providers trails
  each provider's newest features. A native adapter can be added beside it
  without touching the harness.

### The Anthropic adapter is built on genai

It saves writing the wire format, the event-stream parsing, and the assembly
of tool input. The same translation gives other providers almost for free.

### The translation layer stays, for now

About two thirds of the adapter is translation between our types and genai's,
and today it protects nothing: there is no second adapter and no extension
that reads messages. Dropping our data model and using genai's types directly
was considered. It was kept. Revisit if the translation becomes a burden
before it has paid for itself.

## Alternatives considered

- **A hand-written HTTP client** — the first choice, made while the design
  was still synchronous. One endpoint, and full control over new API
  features. More work up front, and the same again for each provider.
- **A community Anthropic SDK** — none was vetted.
- **genai's types as our public API** — the least code. Ties our releases to
  genai's.
- **Message type chosen by the adapter** — history kept in the provider's own
  format, with no translation. Tools and the UI could then not read the
  conversation without a neutral view anyway.

## Consequences

Gaps in genai 0.6.5 for Anthropic, found by reading its source:

- **Thinking blocks are not kept** — they are neither captured from a reply
  nor sent back. Current models think by default, so a tool-use turn sent
  back without them may be rejected.
- **No error flag on tool results** — an error has to say so in its text.
- **Block order is rebuilt** — a reply comes back as text, then tool calls,
  not in the order the model wrote them.
- **Output tokens are over-counted when streaming** — the count from the
  start of the message is added to the final, already cumulative, count.

Update, same day: moved to the 0.7 release candidate. It keeps thinking
blocks and counts output tokens correctly; both are covered by adapter
tests. The error flag and block order are unchanged. The price is a
pre-release dependency, pinned to an exact version.

Further:

- **Checked against the real API only with a model that does not think** —
  thinking is verified against a local fake server, not the real one.
- **New provider features wait** on genai, or on a native adapter.
- **Adapter tests go through the public interface** against a local server
  speaking the wire format, so they survive replacing genai.
