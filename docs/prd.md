# PRD: haarniska

**Status:** Draft
**Owner:** Olli Varila

## 1. Overview

A coding agent harness in Rust, shipped as a library. Users build their own
agent by combining the core with plugins, which are plain Rust crates.

Existing extensible harnesses (for example pi, in TypeScript) start slowly, and
each third-party extension makes startup slower. This project keeps the
flexibility while making startup cost independent of how many plugins are used.

Users are experienced developers who want to shape their agent precisely and
are comfortable writing Rust.

## 2. Functional requirements

- Agent loop - send a prompt to a model, stream the reply, run the
  tool calls it makes, and repeat until the model is done.
- Built-in tools - read, write, and edit files; run shell commands.
- Providers - talk to model APIs through one provider interface.
- Terminal UI - interactive prompt, streamed replies, tool calls and
  results, scrolling back through the session.
- Plugins - add tools, and hook into the agent loop to block, change, or
  observe what happens.
- Consent - ask the user before chosen tool calls run, such as shell
  commands.
- Project instructions - read `AGENTS.md` from the project and follow it.
- Skills - find the skills available to the project, tell the model what
  each is for, and load one when the task calls for it.

## 3. Architectural characteristics

- **Startup performance** — under 50 ms to the prompt with 20 plugins,
  excluding network. Cost does not grow with plugin count.
- **Runtime performance** — harness work per turn is negligible next to model
  latency.
- **Extensibility** — tools, hooks, providers, and UI can all be
  added or replaced. Built-in features use the same public API as plugins.
- **Simplicity** — a small core API; a new tool plugin takes under 50 lines.
- **Stability** — API changes follow semver and rarely break plugins.
- **Build speed** — a one-line plugin change rebuilds in under 10 s.
- **Fault tolerance** — a failing plugin or tool does not end the session.

## 4. Non-goals

Not in v1:

- A default binary (an example shows a full setup instead)
- `AGENTS.md` files outside the project root (parent, nested, or user-wide);
  planned next, so the design must leave room for them
- Slash commands
- Installing or reloading plugins at runtime
- Plugins written in other languages, including pi extensions
- A permission sandbox (run the process in a container instead). Asking for
  consent is a check before a call, not a boundary around the agent
- Context compaction
- Session branching
- An RPC / JSON mode for embedding
- Custom keybindings
- A GUI or web frontend

## 6. User stories

1. As a developer, I want a kitchen sink example that works out of the box
   so I can easily test the tool out.

2. As a developer, I want to build my own agent from the library
   with the plugins I choose, so that it works exactly how I want.

3. As a plugin author, I want to add a tool the model can call, so
   that the agent can do new things.

4. As a developer with many plugins, I want startup to stay instant,
   so that plugins never make the tool slower to open.

5. As a developer, I want the agent to follow my project's `AGENTS.md`, so
   that I do not repeat its rules in every prompt.

6. As a developer, I want the agent to use my skills, so that it handles
   recurring tasks the way I have written down.

7. As a developer, I want to hook into the agent loop, so that I can
   block, change, or record what the agent does, such as refusing a
   dangerous shell command.

8. As a developer, I want to scroll back through the session, so that I can
   read a long reply or an earlier tool result.

9. As a developer, I want to approve shell commands and file changes before
   they happen, so that the agent cannot surprise me.

## 7. Acceptance criteria

- One can run example agent setup and perform simple actions using API key only.
- Custom plugins can be added to the harness.
- Misbehaving plugin does not crash the application.
- Startup to prompt is under 50 ms. Startup time displayable in UI.
- A rule in the project's `AGENTS.md` changes what the agent does.
- A skill is used when a prompt matches it, and not loaded otherwise.
- A hook can stop a tool call before it runs, and the model is told why.
- A hook can change a tool call's input or result.
- A failing hook does not end the session.
- A tool call that needs consent does not run until the user allows it. A
  denied call does not run, and the model is told.
- With no way to ask the user, a call that needs consent does not run.
- Earlier output can be scrolled back to, also while a turn is running, and
  the view returns to following new output.
