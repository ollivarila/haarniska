# PRD: haarniska

**Status:** Draft
**Owner:** Olli Varila

## 1. Overview

A coding agent harness in Rust, shipped as a library. Users build their own
agent by combining the core with plugins, which are plain Rust crates. A
default binary serves users who do not want to customize.

Existing extensible harnesses (for example pi, in TypeScript) start slowly, and
each third-party extension makes startup slower. This project keeps the
flexibility while making startup cost independent of how many plugins are used.

Primary users are experienced developers who want to shape their agent
precisely and are comfortable writing Rust. Secondary users run the default
binary as is.

## 2. Functional requirements

- Agent loop - send a prompt to a model, stream the reply, run the
  tool calls it makes, and repeat until the model is done.
- Built-in tools - read, write, and edit files; run shell commands.
- Providers - talk to model APIs through one provider interface.
- Terminal UI - interactive prompt, streamed replies, tool calls and
  results, slash commands.
- Plugins - add tools and slash commands, and hook into the agent loop
  to block, change, or observe what happens.

## 3. Architectural characteristics

- **Startup performance** — under 50 ms to the prompt with 20 plugins,
  excluding network. Cost does not grow with plugin count.
- **Runtime performance** — harness work per turn is negligible next to model
  latency.
- **Extensibility** — tools, commands, hooks, providers, and UI can all be
  added or replaced. Built-in features use the same public API as plugins.
- **Simplicity** — a small core API; a new tool plugin takes under 50 lines.
- **Stability** — API changes follow semver and rarely break plugins.
- **Build speed** — a one-line plugin change rebuilds in under 10 s.
- **Fault tolerance** — a failing plugin or tool does not end the session.

## 4. Non-goals

Not in v1:

- Installing or reloading plugins at runtime
- Plugins written in other languages, including pi extensions
- A permission sandbox (run the process in a container instead)
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

## 7. Acceptance criteria

- One can run example agent setup and perform simple actions using API key only.
- Custom plugins can be added to the harness.
- Misbehaving plugin does not crash the application.
- Startup to prompt is under 50 ms. Startup time displayable in UI.
