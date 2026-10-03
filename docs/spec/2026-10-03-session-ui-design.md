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
- **Layout** — a scrolling transcript, an input line, a status line. The
  input line spans the window, between two horizontal rules.
- **Transcript** — the user's prompts, the reply as it streams, each tool
  call with its result, failures.
- **Status line** — startup times and token usage.
- **Two startup times** — to the first frame, when the user can start
  typing, and to the agent being ready, when the harness first waits for
  input. Both count from when the program started.
- **Keys** — Enter sends the prompt, Esc cancels the running turn, Ctrl-C
  and Ctrl-D quit.
- **Terminal state** — the screen is restored on quit and when the UI itself
  crashes.

### Consent

Example: the agent wants to run `git push` and a hook asked for consent.

```
─ Allow shell? ──────────────────────────────────────
git push
pushes to the remote
─ y allow, n deny, a always allow shell ─────────────
```

- **Where** — the question takes the place of the input line until it is
  answered.
- **Shows** — the tool, its input, and each reason a hook gave.
- **Keys** — `y` allows, `n` denies, `a` always allows this tool for the
  session. Esc cancels the whole turn, as always. Enter does nothing here:
  it is too easy to press out of habit.
- **The rest keeps working** — scrolling, and the transcript, which still
  shows what came before.
- **Provided to the harness** — the UI hands out an approver before the
  agent is built. See the [hooks spec](2026-10-03-hooks-design.md).

### Scrolling

Example: a reply runs to 200 lines and the user wants the start of it.

- **Following** — by default the transcript shows its newest lines and moves
  as the reply streams.
- **Scrolling back** — Page Up and Page Down move by a screen; the mouse
  wheel moves by a few lines. Scrolling up stops following: new output no
  longer moves what is on screen.
- **Selecting text** — the UI takes the mouse to see the wheel, so the
  terminal's own selection needs its bypass key, usually Shift.
- **Back to following** — scrolling down to the end, or pressing End.
  Sending a prompt also returns to the end.
- **Shown in the status line** — when not following, the status line says
  so, and how to return.
- **Whole session** — everything since the session started can be reached.
- **While a turn runs** — scrolling works the same; it never blocks or
  cancels the turn.

### Markdown

Example: the model answers with a heading, a list, and a Rust code block.

- **Replies only** — the model's reply is shown as markdown. Prompts, tool
  calls, and tool results stay as typed.
- **Shown** — headings, emphasis, lists, quotes, links, tables, inline code,
  and code blocks. The markup itself is not shown. A link shows its address
  after its text.
- **Code blocks** — highlighted by language when the language is known,
  otherwise one colour. The fence lines are left out.
- **Wrapping** — text breaks at spaces. Code is cut at the window width, so
  its layout holds. Tables are laid out to the window width. The rows a
  list item or quote continues on are not indented.
- **Around a code block** — the text before and after is rendered
  separately. A list with a code block in it shows as two lists.
- **Colours, no backgrounds** — a background ends where each line's text
  does and shows as ragged bars.
- **While streaming** — the reply so far is rendered on each update.
  Unfinished markup shows as best it can and settles when the rest arrives.

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
3. How much of a tool result to show, and a way to expand it.
4. Multi-line input and prompt history.
5. Must UIs be movable between threads?
6. Highlighting adds a heavy dependency for every user of the crate. Should
   it be optional?
