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
- **Before: block, ask, or change** — a hook can stop the call, make it
  wait for the user's consent, or change its input. It cannot change which
  tool is called.
- **Hooks decide, the harness asks** — a hook only says that a call needs
  consent. Asking the user is the harness's job, through an approver the
  caller provides. Hooks never hold the UI.
- **No one to ask means no** — without an approver, a call that needs
  consent is blocked.
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
  may change the input in place. It answers continue, block, or ask.
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
2. If a hook asked, the user is asked. A deny stops here.
3. The tool is found, the input is parsed, and the tool runs.
4. Each hook's after point, in order.
5. The result goes to the model.

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

## 4. Asking the user

Example: let the agent work freely, but confirm anything that leaves the
machine.

```rust
impl Hook for AskBeforePush {
    async fn before_tool(&self, cx: &ToolCx, tool: &str, input: &mut Value) -> Decision {
        let command = input["command"].as_str().unwrap_or_default();
        if tool == "shell" && command.starts_with("git push") {
            return Decision::Ask("pushes to the remote".into());
        }
        Decision::Continue
    }
}
```

```rust
let tui = Tui::new(started)?;

let agent = haarniska::builder()
    .with_inference(inference)
    .with_default_tools()
    .with_hook(AskBefore::tools(["shell", "write", "edit"]))
    .with_hook(AskBeforePush)
    .with_approver(tui.approver())
    .build();
```

```
model     → ToolCall(shell, {command: "git push"})
hooks     → Ask("shell needs consent"), Ask("pushes to the remote")
harness   → approver: shell, git push, both reasons
user      → allow
harness   → runs shell
```

- **Ask** — the call runs only if the user agrees. The hook gives the
  reason shown to the user.
- **Approver** — whatever can put the question to a person. The terminal UI
  provides one. A caller without a UI can provide their own, or none.
- **What the user is shown** — the tool, its input after every hook's
  changes, and each reason.
- **Answers** — allow, deny, or always allow this tool for the rest of the
  session.
- **Always** — remembered by the harness per tool name. Later calls to that
  tool that would be asked about run without asking. A block still blocks.
- **Deny** — error result saying the user declined. The tool does not run
  and the turn continues.
- **A default to start from** — a ready-made hook asks before the tools the
  caller names. Nothing asks unless the caller adds a hook that does.

## 5. Several hooks

- **Before** — each hook sees the input as the previous one left it. A
  block stops there; later hooks are not asked. After an ask, later hooks
  still run, since one of them may block.
- **Asked once** — when hooks have run and none blocked, the user is asked a
  single time, however many hooks asked.
- **After** — each hook sees the result as the previous one left it. All of
  them run.
- **Same order both times** — registration order.

## 6. Error handling

- **Hook blocks** — error result: the call was blocked, and why. The tool
  does not run. The turn continues.
- **Hook crashes before a call** — treated as a block. A guard that fails
  must not let the call through. The error result says a hook crashed.
- **Hook crashes after a call** — the result is passed on as it was before
  that hook. Later hooks still run.
- **Hook never returns** — the turn waits. Cancelling the turn stops it, as
  with a tool.
- **Consent needed, no approver** — treated as a block. The error result
  says the call needs consent and there is no one to ask.
- **User does not answer** — the turn waits. Cancelling the turn withdraws
  the question.
- **Approver crashes** — treated as a deny.

## Open questions

1. More points: before a model call, when a prompt is sent, when a turn
   ends.
2. Consent for a pattern, such as every `cargo` command, and consent that
   outlasts the session.
3. Should a hook say which tools it applies to, instead of checking the name
   itself?
4. Should a hook be able to answer a call itself, with a result, without the
   tool running?
5. A plugin type that bundles tools and hooks.
6. Letting the user say why they declined, for the model to read.
