# Spec: Public API

**Date:** 2026-10-03
**Status:** Draft
**Owner:** Olli Varila
**PRD:** [PRD](../prd.md)

## Overview

The shape of the library's public API: what a user composes, what they get
back, and who drives what. It covers the agent loop, providers, plugins, and
the UI boundary from the [PRD](../prd.md). Contents of each part (tool
interface, event types, errors) are later specs.

## Locked decisions

- **Agent = harness + model** — the harness is everything around the model:
  the loop, tools, plugins. Adding a provider makes it an agent. The agent is
  the one thing a user builds.
- **One builder** — sets the provider and the plugins, then yields an agent.
- **Harness owns both loops** — the agentic loop (one turn) and the session
  loop (interactive). There is no separate runtime.
- **UI is outside the agent** — the agent never stores a UI. A UI is handed
  over only to run an interactive session.
- **UI is passive** — the harness drives the UI. The UI does not drive the
  agent.
- **Headless by default** — an agent is fully usable with no UI.
- **Plugins belong to the harness** — they extend it; they are not a peer of
  it.
- **No async** — blocking calls and plain threads. Nothing in scope needs
  more, and it keeps the API small and builds fast.
- **No hooks yet** — plugins cannot yet block, change, or observe the loop.

## 1. Model

Four concepts, one composition:

```
Provider ─┐
          ├─ builder ─→ Agent ──→ one turn   (headless)
Plugins  ─┘               │
                          └─ + Ui ─→ session (interactive)
```

- **Provider** — a model API behind one interface.
- **Plugin** — an extension to the harness.
- **Agent** — harness plus provider. Holds the conversation.
- **Ui** — a frontend the harness can drive. The terminal UI is one.

Dependencies point one way: UIs and providers know nothing of each other, and
the harness knows a UI only through the Ui interface.

## 2. Building an agent

```rust
let agent = Agent::builder()
    .provider(provider)
    .plugin(plugin_a)
    .plugin(plugin_b)
    .build();
```

- **Provider** — exactly one.
- **Plugins** — any number, added one by one.
- **Cheap** — building does no I/O, so startup cost does not grow with the
  number of plugins.

## 3. Headless: one turn

Example: a script wants one answer and no terminal.

```rust
for event in agent.prompt("fix the bug") {
    // text, tool activity, ...
}
```

- **A turn** — send the prompt, stream the reply, run the tool calls the
  model makes, repeat until the model is done.
- **Events are pulled** — the caller asks for the next event; the loop
  advances only then. No threads, no callbacks.
- **Stopping** — a caller that stops pulling stops the turn.
- **State** — the conversation stays in the agent, so the next prompt
  continues it.

## 4. Interactive: a session

```rust
agent.run(Tui::new());
```

- **Session loop** — take input from the UI, run a turn, send its events to
  the UI, repeat until the UI ends the session.
- **Ownership** — the UI is an argument to the session, not a part of the
  agent. An agent with no UI is a valid agent, and a missing UI cannot be a
  runtime error.
- **Replaceable** — any frontend that implements the Ui interface can be
  passed in. The terminal UI has no special access.

## 5. Error handling

- **Tool fails** — the session continues. The failure is reported to the
  model and shown as an event.
- **Plugin misbehaves** — the session continues.
- **Provider fails mid-turn** — the turn ends with an error event. The
  session continues.

How errors are typed and surfaced is open (see below).

## Open questions

1. What does the Provider interface look like (request, streamed reply)?
2. What does a plugin register, and how? Tools first; slash commands later.
3. How do tools take input: typed, or raw JSON with a hand-written schema?
4. Which events exist?
5. What does the Ui interface look like (input in, events out)?
6. How is a running turn cancelled, including a blocked tool?
7. Error type, and which calls can fail.
8. Is the terminal UI optional at build time?
