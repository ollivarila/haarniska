# ADR 0001: The harness owns both loops; the UI is passive

**Date:** 2026-10-03
**Status:** Accepted
**Owner:** Olli Varila
**Related:** [PRD](../prd.md) · [Spec](../spec/2026-10-03-public-api-design.md)

## Context

An agent has two loops. The agentic loop is one turn: call the model, run the
tools it asks for, repeat until it is done. The session loop is the
interactive one: take input, run a turn, show what happened, repeat.

Two users show the tension. A script wants one answer and has no terminal. A
terminal app wants a full session. Something has to run each loop, and the UI
has to sit somewhere.

## Decision

### The harness runs both loops

The agentic loop is what makes a harness more than a list of tools, and it is
where hooks will attach later. The session loop is thin glue on top of it. It
does not earn a concept of its own.

### The UI is passive

The harness drives the UI: it asks for input and sends events. A UI never
holds the agent and never decides when a turn runs.

### The UI is handed over when a session starts

The UI is not set when the agent is built and is not stored in it. An agent is
a harness plus a model, and stays that. Headless use needs no UI at all, and a
missing UI cannot be a runtime error.

## Alternatives considered

- **UI set on the builder** — the first idea: one builder for everything, one
  call to run. The UI becomes optional state, so running without one fails at
  runtime, and the agent turns into harness plus model plus frontend. It
  would let extensions replace the UI through the builder; nothing needs
  that yet, and it can be added later.
- **UI drives the agent** — the UI holds the agent, sends prompts, and reads
  events. Smallest API, since no UI interface is needed. Rejected because
  control flow would live in each frontend instead of in the harness.
- **A separate runtime** — a third piece owning the session loop and the
  wiring between agent and UI. Rejected because it would be a few dozen lines
  of glue beside a harness that holds everything else. It pays off with a
  second driver, such as an RPC mode, which is a v1 non-goal.
- **A runtime that also runs the agentic loop** — leaves the harness as a
  tool registry and splits the loop from what will hook into it.

## Consequences

- A UI interface exists and has to be designed.
- Every frontend gets the same session behaviour without reimplementing it.
- Headless use is a first-class path, not a special mode.
- Extensions cannot swap the UI through the builder.
- If a second driver appears, the session loop can move into its own piece
  without changing the public API.
