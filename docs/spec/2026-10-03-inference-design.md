# Spec: Inference

**Date:** 2026-10-03
**Status:** Draft
**Owner:** Olli Varila
**PRD:** [PRD](../prd.md)

## Overview

Inference is how the harness talks to a model. The harness speaks one neutral
interface; each model API gets an adapter that translates to and from its own
wire format. This spec sets where that boundary sits and the data that crosses
it. It builds on the [public API spec](2026-10-03-public-api-design.md) and
covers the "Providers" requirement of the [PRD](../prd.md). Anthropic is the
first adapter.

## Locked decisions

- **The boundary is one model call** — a request goes in, a stream of chunks
  comes out. The loop around it belongs to the harness.
- **Neutral data model** — the harness never sees provider-shaped data.
- **Harness owns the conversation** — an adapter keeps no history. It gets the
  full message list on every call.
- **Model settings live on the adapter** — model, effort, thinking, token
  limits. They are provider-specific and are not part of the request.
- **Text streams, the reply arrives whole** — text comes as deltas for
  display. The complete reply, including tool calls, comes once at the end.
- **Built on genai, async** — there is no official Rust SDK for the
  Anthropic API. The adapter uses the genai crate, hidden behind our
  interface. See [ADR 0003](../adr/0003-own-inference-interface-genai-behind-it.md).
- **Prompt caching is the adapter's job** — the neutral request says nothing
  about caching. Each adapter caches the way its provider allows.

## 1. Boundary

```
harness ── Request (system, messages, tools) ──▶ adapter ──▶ HTTP
harness ◀── Chunk stream (text ..., done)    ─── adapter ◀── SSE
```

| Harness                              | Adapter                               |
| ------------------------------------ | ------------------------------------- |
| Conversation history                 | Auth, HTTP, retries                   |
| Tool registry, running tools         | Wire format, both directions          |
| The loop: continue, stop, cancel     | Stream parsing, assembling tool input |
| System prompt                        | Model settings                        |

The test for the boundary: a second adapter can be written without changing
the harness.

- **Not higher** — if an adapter ran the whole turn, every adapter would
  reimplement the tool loop.
- **Not lower** — if an adapter were only transport, the harness would have
  to know each wire format.

## 2. Data model

Sent to the adapter:

```rust
struct Request<'a> {
    system: &'a str,
    messages: &'a [Message],
    tools: &'a [ToolSpec],
}

struct ToolSpec { name: String, description: String, input_schema: Value }
```

The conversation:

```rust
enum Message {
    User(String),
    Assistant(Vec<Block>),
    ToolResults(Vec<ToolResult>),
}

enum Block {
    Text(String),
    ToolCall(ToolCall),
    Opaque(Value),
}

struct ToolCall   { id: String, name: String, input: Value }
struct ToolResult { call_id: String, output: String, is_error: bool }
```

Returned by the adapter:

```rust
enum Chunk {
    Text(String),   // a delta, for display only
    Done(Reply),    // always last
}

struct Reply { content: Vec<Block>, stop: StopReason, usage: Usage }

enum StopReason { EndTurn, ToolUse, MaxTokens, Refusal }

struct Usage { input_tokens: u64, output_tokens: u64 }
```

- **One variant per kind of message** — a tool result cannot appear in an
  assistant message, and a tool call cannot come from the user.
- **Tool results travel together** — all results of one round are one
  message. Some APIs require this; others can split it up.
- **The reply is authoritative** — the harness appends the reply's content to
  the conversation as is. It does not rebuild it from deltas.
- **Usage is minimal** — the two counts every provider has. Finer detail
  stays in the adapter.
- **Open to growth** — every enum can gain variants without breaking
  adapters or tools.

## 3. A turn, traced

The user asks to fix a bug; the model reads a file, then answers.

```
harness → adapter   [User("fix the bug")]
adapter → harness   Text("Let me look."), Done(Reply {
                      content: [Text("Let me look."), ToolCall(read, a.rs)],
                      stop: ToolUse })
harness             runs read, appends Assistant(...) and ToolResults([...])
harness → adapter   [User, Assistant, ToolResults]
adapter → harness   Text("Fixed: ..."), Done(Reply {
                      content: [Text("Fixed: ...")], stop: EndTurn })
harness             appends Assistant(...); the turn is over
```

## 4. Opaque blocks

Some reply content is meaningful only to the provider but must be sent back
unchanged on the next call. Thinking blocks are the case today.

- **Adapter** — writes them into the reply and reads them back from history.
- **Harness** — stores them in order and never looks inside.
- **Unknown blocks** — an adapter ignores opaque blocks it does not
  recognise.

## 5. Anthropic adapter

- **Configuration** — API key from the environment; model and other settings
  set when it is created.
- **Request** — maps messages and tool specs to the Messages API. A
  ToolResults message becomes one user message holding every result.
- **Stream** — text deltas are passed on as they arrive. Tool input arrives
  in fragments; the adapter joins and parses them before the reply is done.
- **Stop reasons** — mapped to the four neutral ones.
- **Prompt caching** — the last message of every request is marked as a
  cache point. The provider then keeps everything up to it (tools, system
  prompt, conversation) and the next request reads it back at a fraction of
  the price. Each request moves the point forward.

Caching in practice:

- **Needs a stable start** — it only works while the tools and system prompt
  stay the same through a session. They do.
- **Short conversations are not cached** — the provider ignores a cache point
  below a minimum size.
- **Five minutes** — the provider's default lifetime. After a longer pause
  the next request pays full price once.

## 6. Bedrock adapter

Claude on Amazon Bedrock, behind the `bedrock-inference` cargo feature. See
[ADR 0004](../adr/0004-bedrock-adapter-on-aws-sdk.md).

- **Transport** — `InvokeModelWithResponseStream` on the AWS SDK, with the
  Messages API as the body. Request and stream map the same as the Anthropic
  adapter's.
- **Configuration** — model or inference profile ID set when it is created.
  AWS configuration is loaded on the first call: profile from `AWS_PROFILE`
  (or set explicitly), region from `AWS_REGION`.
- **Auth refresh** — an optional shell command. When a call fails before any
  chunk because credentials are missing or expired, the command runs once,
  the configuration is loaded again, and the call is retried. Its output is
  captured, not shown.
- **Prompt caching** — the same cache point as the Anthropic adapter.
- **Opaque blocks** — thinking blocks are kept as the provider's raw block.
  Opaque blocks from other adapters are not sent.
- **Usage** — input tokens include cache reads and writes.

## 7. Error handling

- **Cannot start** — bad key, no network, rejected request. The call fails
  before any chunk; the turn ends with an error and the session continues.
- **Stream breaks mid-reply** — the stream ends with an error instead of a
  reply. Partial text was shown but is not added to the conversation.
- **Max tokens** — a normal reply with that stop reason. The harness decides
  what to do; it is not treated as done.
- **Refusal** — a normal reply with that stop reason. The turn ends and the
  user is told.
- **Malformed tool input** — the adapter cannot parse a tool call's input.
  The call is kept and answered with an error result, so the model can retry.

## Open questions

1. What does the error type look like, and does it say whether a retry is
   worthwhile?
2. Does the Anthropic adapter live in this crate behind a build flag, or in
   its own crate? The Bedrock adapter is behind a build flag.
3. Who retries, and how often?
4. User messages beyond text (images, files).
5. Do opaque blocks need a provider tag, for switching providers mid-session?
6. A longer cache lifetime, and showing cache use to the user.
7. Should thinking summaries and tool calls in progress be streamed for
   display?
