# ADR 0002: Async on Tokio

**Date:** 2026-10-03
**Status:** Accepted
**Owner:** Olli Varila
**Related:** [PRD](../prd.md) · [Spec](../spec/2026-10-03-public-api-design.md)

## Context

The first decision was to stay synchronous: blocking calls, plain threads,
channels between the loop and the UI. Nothing in scope needed more, the API
was smaller, builds were faster, and there was no runtime to start.

That changed when looking at what tools will be built on. An MCP client, an
HTTP-calling tool, and a language-server client are all async in Rust. A tool
author inside a synchronous harness would have to start their own runtime and
block on it in every call.

## Decision

### The public API is async

Turns are streams of events, tools are async, and model calls are async.

- **Tools decide it** — the libraries tool authors will reach for are async.
- **The break only goes one way** — moving to async later would break every
  tool. Now there are none to break.
- **Cancelling is dropping** — interrupting a blocked read or a running
  command was the hard part of the synchronous design. With async, a turn is
  cancelled by dropping it.

### Tokio is named as the runtime

The API does not try to work on any runtime. Tools need processes, files, and
timers, and a neutral API would make each of those harder for little gain.

## Alternatives considered

- **Synchronous, with threads** — the original choice, for the reasons above.
  It still suits tools we write ourselves, since blocking work can run on a
  blocking pool. It does not help authors whose dependencies are async.
- **Async, runtime-agnostic** — keeps Tokio out of the public API. Rejected as
  friction without a user who needs another runtime.

One argument raised for async was that the Anthropic SDK is async. There is no
official Rust SDK, so this did not decide it.

## Consequences

- Tokio and an async HTTP stack are dependencies. Clean builds are slower;
  rebuilding after a small change is not.
- Startup time is not affected in practice.
- Interfaces stored as trait objects need boxed futures.
- Whether things must be movable between threads now has to be answered.
  Tools must be; agents and UIs are still open.
