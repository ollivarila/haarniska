# ADR 0004: Bedrock adapter on the AWS SDK, not genai

**Date:** 2026-10-04
**Status:** Accepted
**Owner:** Olli Varila
**Related:** [Spec](../spec/2026-10-03-inference-design.md) · [ADR 0003](0003-own-inference-interface-genai-behind-it.md)

## Context

Claude should also be usable through Amazon Bedrock. Prompt caching must
work, and credentials come from an AWS profile that may need a login to
refresh.

genai 0.7.0-rc.1 has Bedrock adapters, on the Converse API. Read from its
source:

- **No prompt caching** — cache points are dropped from the request.
- **Thinking blocks are not sent back** — a tool-use turn of a thinking
  model loses them.

## Decision

### A native adapter on InvokeModel, with a Messages API body

`InvokeModelWithResponseStream` takes the Anthropic Messages API body as is,
and streams Anthropic's own events back. Caching, thinking blocks and the
error flag on tool results all work.

The official `aws-sdk-bedrockruntime` crate does the rest: request signing,
the credential chain (profiles, SSO, credential processes), region, and the
event-stream encoding.

### Behind the `bedrock-inference` cargo feature

The AWS crates are large. Users of the Anthropic API alone should not build
them.

### Auth refresh is a user command

When credentials are missing or expired, an optional shell command (e.g.
`aws sso login`) is run once and the call is retried. The AWS configuration
is loaded again, so new credentials are picked up.

## Alternatives considered

- **genai's Bedrock adapters** — no code, but no caching.
- **Converse through the AWS SDK** — typed, but Bedrock's own format: it
  trails Anthropic's features and needs a second translation of thinking and
  cache points.
- **Bedrock's Messages API endpoint (Mantle)** — the same wire format as
  Anthropic's API, but there is no Rust client for it.

## Consequences

- **Two Messages API translations** — the Anthropic adapter's, through
  genai, and this one. The Anthropic adapter could move onto this one and
  drop genai.
- **No model constants** — Bedrock model IDs depend on region and inference
  profile; the caller passes the ID.
- **Checked against a local fake server only** — not yet against Bedrock.
