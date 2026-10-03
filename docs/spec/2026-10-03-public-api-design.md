# Spec: Public API

**Date:** 2026-10-03
**Status:** Draft
**Owner:** Olli Varila
**PRD:** [PRD](../prd.md)

## Overview

The shape of the library's public API: what a user composes, what they get
back, and who drives what. It covers the agent loop, inference, plugins, and
the UI boundary from the [PRD](../prd.md). Contents of each part (tool
interface, event types, errors) are later specs.

## Locked decisions

- **Agent = harness + model** — the harness is everything around the model:
  the loop, tools, plugins. Adding inference makes it an agent. The agent is
  the one thing a user builds.
- **One builder** — sets the inference and the plugins, then yields an agent.
- **Harness owns both loops** — the agentic loop (one turn) and the session
  loop (interactive). There is no separate runtime.
- **UI is outside the agent** — the agent never stores a UI. A UI is handed
  over only to run an interactive session.
- **UI is passive** — the harness drives the UI. The UI does not drive the
  agent.
- **Headless by default** — an agent is fully usable with no UI.
- **Plugins belong to the harness** — they extend it; they are not a peer of
  it.
- **Async, on Tokio** — plugins will depend on async libraries (HTTP clients,
  MCP, language servers), and changing later would break every plugin.
  Cancelling a turn is dropping it.
- **No hooks yet** — plugins cannot yet block, change, or observe the loop.

## 1. Model

Four concepts, one composition:

```
Inference ─┐
           ├─ builder ─→ Agent ──→ one turn   (headless)
Plugins   ─┘               │
                           └─ + Ui ─→ session (interactive)
```

- **Inference** — a model API behind one interface. See the
  [inference spec](2026-10-03-inference-design.md).
- **Plugin** — an extension to the harness.
- **Agent** — harness plus inference. Holds the conversation.
- **Ui** — a frontend the harness can drive. The terminal UI is one.

Dependencies point one way: UIs and inference know nothing of each other, and
the harness knows a UI only through the Ui interface.

## 2. Building an agent

```rust
let agent = haarniska::builder()
    .inference(inference)
    .plugin(plugin_a)
    .plugin(plugin_b)
    .build();
```

- **Inference** — exactly one. Leaving it out, or setting it twice, does not
  compile.
- **Plugins** — any number, added one by one.
- **Cheap** — building does no I/O, so startup cost does not grow with the
  number of plugins.

## 3. Headless: one turn

Example: a script wants one answer and no terminal.

```rust
let mut events = pin!(agent.prompt("fix the bug"));
while let Some(event) = events.next().await {
    // text, tool activity, ...
}
```

- **A turn** — send the prompt, stream the reply, run the tool calls the
  model makes, repeat until the model is done.
- **Events are a stream** — the caller awaits the next event; the loop
  advances only then. No callbacks.
- **Stopping** — a caller that drops the stream cancels the turn.
- **State** — the conversation stays in the agent, so the next prompt
  continues it.

## 4. Interactive: a session

```rust
agent.run(Tui::new()).await;
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
- **Inference fails mid-turn** — the turn ends with an error event. The
  session continues.

How errors are typed and surfaced is open (see below).

## Open questions

1. Must agents, plugins, and UIs be movable between threads?
2. What does a plugin register, and how? Tools first; slash commands later.
3. How do tools take input: typed, or raw JSON with a hand-written schema?
4. Which events exist?
5. What does the Ui interface look like (input in, events out)?
6. How is a running turn cancelled, including a blocked tool?
7. Error type, and which calls can fail.
8. Is the terminal UI optional at build time?
