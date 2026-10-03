# Spec: Hooks

**Date:** 2026-10-03
**Status:** Draft
**Owner:** Olli Varila
**PRD:** [PRD](../prd.md)

## Overview

A hook is code that runs at fixed points of the agent loop and can block,
change, or record what happens there. This spec covers the points that exist,
how a hook is written and registered, and what happens when several hooks or
a failing hook are involved. It covers the hook part of the "Plugins"
requirement of the [PRD](../prd.md) and builds on the
[tools spec](2026-10-03-tools-design.md).

## Locked decisions

- **Two points, both around a tool call** — before it runs and after it
  runs. More points can be added without breaking existing hooks.
- **Before: block or change** — a hook can stop the call, or change its
  input. It cannot change which tool is called.
- **After: change** — a hook can change the result the model gets.
- **Recording is just not changing** — a hook that only looks is a hook that
  leaves everything as it is. There is no separate observer.
- **Blocking is an answer, not a failure** — a blocked call gives the model
  an error result with the hook's reason, and the turn goes on.
- **Registered on the builder** — one by one, like tools. Still no plugin
  type: a plugin is a crate that provides tools and hooks.
- **In order** — hooks run in the order they were registered, one at a time.
- **A crashing guard blocks** — if a hook crashes before a tool call, the
  call does not run.
- **Async, with a context** — like tools.

## 1. Writing a hook

Example: refuse a dangerous shell command.

```rust
struct NoForceDelete;

impl Hook for NoForceDelete {
    async fn before_tool(&self, cx: &ToolCx, tool: &str, input: &mut Value) -> Decision {
        let command = input["command"].as_str().unwrap_or_default();
        if tool == "shell" && command.contains("rm -rf") {
            return Decision::Block("rm -rf is not allowed in this project".into());
        }
        Decision::Continue
    }
}
```

Example: keep secrets out of what the model reads.

```rust
impl Hook for RedactSecrets {
    async fn after_tool(&self, cx: &ToolCx, call: &ToolCall, result: &mut ToolResult) {
        result.output = redact(&result.output);
    }
}
```

- **Both points are optional** — a hook implements the ones it needs. The
  other does nothing.
- **Before** — gets the tool's name and its input as the model sent it, and
  may change the input in place. It answers continue or block.
- **After** — gets the call and its result, and may change the output and
  whether it counts as an error.
- **Every tool** — a hook sees calls to all tools and picks the ones it
  cares about by name.
- **Context** — the same one tools get.

## 2. Registering

```rust
let agent = haarniska::builder()
    .with_inference(inference)
    .with_default_tools()
    .with_hook(NoForceDelete)
    .with_hook(RedactSecrets)
    .build();
```

- **Any number** — added one by one.
- **Cheap** — an agent without hooks pays nothing for them.
- **Fixed after build** — like tools.

## 3. Where hooks run

A tool call from the model goes through these steps:

1. Each hook's before point, in order. A block stops here.
2. The tool is found, the input is parsed, and the tool runs.
3. Each hook's after point, in order.
4. The result goes to the model.

```
model     → ToolCall(shell, {command: "rm -rf build"})
hook      → Block("rm -rf is not allowed in this project")
harness   → event: tool started (shell, rm -rf build)
            event: tool finished (error: blocked)
harness   → model: error result with the reason
model     → ToolCall(shell, {command: "cargo clean"})
hook      → Continue
harness   → runs shell
```

- **Changed input** — parsed after the hooks, so a hook that breaks the
  input gets the usual "input does not fit" error result.
- **The conversation keeps the model's own call** — a changed input is what
  the tool runs with, but the reply is recorded as the model wrote it.
- **After sees everything** — every result the model will get passes the
  after point: successes, tool errors, and blocked calls.
- **What the UI sees** — the call as it was actually run, after any change,
  and the result as the model gets it. A blocked call is shown as a call
  that failed with the reason.

## 4. Several hooks

- **Before** — each hook sees the input as the previous one left it. The
  first block wins; later hooks are not asked.
- **After** — each hook sees the result as the previous one left it. All of
  them run.
- **Same order both times** — registration order.

## 5. Error handling

- **Hook blocks** — error result: the call was blocked, and why. The tool
  does not run. The turn continues.
- **Hook crashes before a call** — treated as a block. A guard that fails
  must not let the call through. The error result says a hook crashed.
- **Hook crashes after a call** — the result is passed on as it was before
  that hook. Later hooks still run.
- **Hook never returns** — the turn waits. Cancelling the turn stops it, as
  with a tool.

## Open questions

1. More points: before a model call, when a prompt is sent, when a turn
   ends.
2. A hook that asks the user, such as "allow this command?". It needs a way
   to reach the UI.
3. Should a hook say which tools it applies to, instead of checking the name
   itself?
4. Should a hook be able to answer a call itself, with a result, without the
   tool running?
5. A plugin type that bundles tools and hooks.
