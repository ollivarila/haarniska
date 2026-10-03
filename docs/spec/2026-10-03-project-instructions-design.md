# Spec: Project instructions

**Date:** 2026-10-03
**Status:** Draft
**Owner:** Olli Varila
**PRD:** [PRD](../prd.md)

## Overview

A project can tell the agent how to work in it through an `AGENTS.md` file,
and a user can have rules of their own that apply everywhere. This spec
covers how those files are layered, how their content reaches the model, and
how the design leaves room for files found during a session. It covers the
"Project instructions" requirement of the [PRD](../prd.md) and builds on the
[public API spec](2026-10-03-public-api-design.md).

## Locked decisions

- **Layers** — instructions are a stack of layers, each one file's content
  and where it came from. Layers go from general to specific.
- **The caller chooses the layers** — the library has no built-in idea of a
  user-wide file or where it lives. A user-wide file is simply the first
  layer.
- **Later wins, by saying so** — every layer's text reaches the model.
  Nothing is removed. The model is told that a later layer applies where two
  conflict.
- **Loaded by the caller** — reading files is a separate step from building
  the agent. Building stays free of I/O, and a read error is the caller's to
  handle.
- **Opt-in** — an agent has no instructions unless they are passed in.
- **After the system prompt** — layers are added to the end of the system
  prompt, each under the path it came from.
- **`CLAUDE.md` as fallback** — projects set up for Claude Code work without
  a second file.
- **Read once** — at startup. A change to a file takes effect in the next
  session.

## 1. Layers

```rust
let instructions = Instructions::new()
    .layer_file("/home/me/.config/myagent/AGENTS.md")?  // user-wide
    .layer_dir(".")?;                                    // the project
```

- **A file** — any path the caller names. This is how a user-wide file, or
  a team-wide one, is added.
- **A directory** — reads `AGENTS.md` in it; the caller normally passes the
  project root. If there is no `AGENTS.md`, `CLAUDE.md` is read instead.
  Never both.
- **`CLAUDE.md` is read as plain text** — its `@path` imports are not
  followed.
- **Order** — the order layers are added. The first is the most general.
- **Missing or empty** — the layer is skipped. Not an error.
- **Paths as given** — a leading `~` is not expanded.

## 2. How it reaches the model

Example: the system prompt is "You are a coding agent.", the user-wide file
says "Answer briefly.", and the project's says "Always use tabs."

```
You are a coding agent.

Instructions follow, from general to specific. Where they conflict, the
later one applies.

Instructions from /home/me/.config/myagent/AGENTS.md:

Answer briefly.

Instructions from ./AGENTS.md:

Always use tabs.
```

- **Path shown** — the model can tell where a rule comes from.
- **Conflict rule** — stated only when there is more than one layer.
- **Content unchanged** — apart from trimming surrounding blank space.
- **No layers** — the system prompt is left as it is.

## 3. Using it

```rust
let agent = haarniska::builder()
    .with_inference(inference)
    .with_instructions(instructions)
    .build();
```

Passing instructions replaces any passed before. Layering happens in the
instructions, not on the builder.

## 4. Room for more layers

Layers outside the caller's list are planned next (see the PRD's non-goals).
Two kinds, with different needs:

- **Known at startup** — files in the directories between the project root
  and where the agent was started. They become layers after the project
  root's, found by a helper. Nothing else changes.
- **Found during a session** — a file in a nested directory matters only
  once the agent works there. It cannot go in the system prompt: that would
  rewrite it mid-session and throw away the prompt cache. It becomes one
  more layer, given to the model as a message when first needed.

So the design must not assume the layers are fixed when the agent is built.
Today they are.

## 5. Error handling

- **No file** — the layer is skipped; the agent is built as usual.
- **File cannot be read** — permissions, or not valid text. Adding the layer
  fails and the caller decides: stop, or go on without it.
- **Very large file** — passed on in full. See open questions.

## Open questions

1. A size limit, and what happens above it.
2. Should the agent load the project's instructions by default, with a way
   to opt out?
3. Finding the project root when started in a subdirectory.
4. How a nested file is noticed: by the tools that touch files, or by the
   harness watching tool calls?
5. Should the UI show which layers are in effect?
