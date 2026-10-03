# Spec: Session and UI

**Date:** 2026-10-03
**Status:** Draft
**Owner:** Olli Varila
**PRD:** [PRD](../prd.md)

## Overview

A session is the interactive loop: take input, run a turn, show what happens,
repeat. This spec covers the interface between the harness and a UI, how the
session loop uses it, and the terminal UI. It builds on the
[public API spec](2026-10-03-public-api-design.md) and
[ADR 0001](../adr/0001-harness-owns-both-loops.md), and covers the "Terminal
UI" requirement of the [PRD](../prd.md).

## Locked decisions

- **The harness drives** — it asks the UI for input and tells it what
  happens. The UI never holds the agent.
- **Two operations** — wait for input, show an event. Nothing else crosses
  the boundary.
- **Three inputs** — a prompt, cancel, quit.
- **Cancelling is dropping** — the harness stops a turn by dropping it.
- **One turn at a time** — a prompt sent while a turn runs is ignored.
- **The terminal UI renders on its own thread** — a tool that blocks cannot
  freeze the screen or the keyboard.
- **Full screen, built on ratatui.**

## 1. Interface

```rust
trait Ui {
    async fn next(&mut self) -> Input;
    fn show(&mut self, event: Event);
}

enum Input { Prompt(String), Cancel, Quit }
```

- **Waiting for input** — asked between turns, and again during a turn so
  the user can cancel or quit. During a turn the wait is abandoned each time
  an event arrives and then started again, so a UI keeps its state in itself,
  not in the wait.
- **Showing an event** — must return at once. A UI that needs time hands the
  event to something else.
- **Events** — the same ones a headless caller gets from a turn.

## 2. Session loop

```
ui      → harness   Prompt("fix the bug")
harness → ui        Text("Let me look.")
harness → ui        ToolStarted(read a.rs)
harness → ui        ToolFinished(...)
harness → ui        Text("Fixed.")
harness → ui        Done
ui      → harness   Quit
```

1. Wait for input. Quit ends the session; cancel does nothing.
2. On a prompt, start a turn.
3. Until the turn ends, take whichever comes first: the next event, which is
   shown, or input from the UI.
4. Go back to 1.

Events are taken before input when both are ready, so nothing a turn produced
is lost to a late key press.

## 3. Cancelling

```
ui      → harness   Prompt("run the tests")
harness → ui        ToolStarted(shell)
ui      → harness   Cancel
                    the turn is dropped; the shell command is killed
ui      → harness   Prompt("never mind")
```

- **Immediate** — the turn stops where it is, including a running tool.
- **No event** — nothing is sent back. The UI asked, so it knows.
- **Conversation** — a round that did not finish is not recorded. The prompt
  that started the turn stays.

## 4. Terminal UI

```
harness                            render thread
  show(event)  ── channel ──▶        apply event, redraw
  next()      ◀── channel ──         keys → Prompt / Cancel / Quit
```

- **Own thread** — reads keys and draws. The harness side only sends and
  receives on channels, so both operations are cheap and neither blocks.
- **Layout** — a scrolling transcript, an input line, a status line.
- **Transcript** — the user's prompts, the reply as it streams, each tool
  call with its result, failures.
- **Status line** — startup time and token usage.
- **Keys** — Enter sends the prompt, Esc cancels the running turn, Ctrl-C
  and Ctrl-D quit.
- **Terminal state** — the screen is restored on quit and when the UI itself
  crashes.

## 5. Error handling

- **Turn fails** — the failure is shown in the transcript. The session
  continues.
- **Tool crashes** — the harness turns it into an error result. Nothing is
  printed over the screen and the terminal stays as it is.
- **Render thread crashes** — the terminal is restored, the crash is printed,
  and the session ends.
- **No terminal** — the terminal cannot be set up. Creating the UI fails
  before a session starts.

## Open questions

1. Should a prompt sent during a turn be queued instead of ignored?
2. A cancelled or failed turn leaves its prompt in the conversation. Should
   it be removed?
3. Long output: scrolling, and how much of a tool result to show.
4. Multi-line input and prompt history.
5. Rendering markdown in replies.
6. Must UIs be movable between threads?
