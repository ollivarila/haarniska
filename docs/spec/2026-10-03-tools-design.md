# Spec: Tools

**Date:** 2026-10-03
**Status:** Draft
**Owner:** Olli Varila
**PRD:** [PRD](../prd.md)

## Overview

A tool is something the model can call: read a file, run a command. Tools are
how the harness is extended. This spec covers how a tool is written, how it is
registered, and how the harness runs tool calls during a turn. It builds on
the [public API spec](2026-10-03-public-api-design.md) and the
[inference spec](2026-10-03-inference-design.md). The built-in tools
themselves (read, write, edit, shell) are a later spec.

## Locked decisions

- **Tools only** — there is no plugin type for now. A plugin is simply a crate
  that provides tools.
- **Registered on the builder** — one by one, before the agent is built.
- **Typed input** — a tool declares its input as a type. The schema shown to
  the model is derived from it, and the harness parses the input before the
  tool runs.
- **Text output** — a tool returns text, or an error.
- **Failure never ends the turn** — every failure becomes an error result
  the model can react to.
- **One call at a time** — calls in the same round run in the order the
  model made them.
- **Context argument** — every call receives a context from the harness, so
  more can be passed later without breaking tools.

## 1. Writing a tool

```rust
#[derive(Deserialize, JsonSchema)]
struct ReadInput {
    /// Path of the file to read.
    path: String,
}

struct Read;

impl Tool for Read {
    type Input = ReadInput;

    const NAME: &str = "read";
    const DESCRIPTION: &str = "Read a file and return its contents.";

    async fn call(&self, cx: &ToolCx, input: ReadInput) -> Result<String, ToolError> {
        Ok(tokio::fs::read_to_string(cx.cwd().join(input.path)).await?)
    }
}
```

- **Name and description** — what the model sees when choosing a tool.
- **Input** — any type that can be read from JSON and describe its own
  schema. Field comments become field descriptions for the model.
- **Result** — text on success. I/O errors can be returned with `?`; any
  other error, or a plain message, is wrapped.
- **Fixed at compile time** — name, description, and input shape are part of
  the tool's type. Tools only known at run time are an open question.
- **Size** — a tool like the one above is about 20 lines.

## 2. Registering

```rust
let agent = haarniska::builder()
    .inference(inference)
    .tool(Read)
    .tool(Shell::new())
    .build();
```

- **Any number** — added one by one.
- **Cheap** — registering does no I/O; startup cost does not grow with the
  number of tools.
- **Fixed after build** — the set of tools does not change during a session.

## 3. A round of tool calls

A reply that stops for tool use holds one or more tool calls. For each call,
in order, the harness:

1. finds the tool by name,
2. parses the input into the tool's input type,
3. announces the call as an event,
4. runs the tool,
5. announces the result as an event.

When every call has a result, the harness adds them to the conversation as
one message and calls the model again.

```
model     → ToolCall(read, {path: "a.rs"}), ToolCall(read, {path: "b.rs"})
harness   → event: tool started (read, a.rs)
            runs read
            event: tool finished (ok)
            event: tool started (read, b.rs)
            runs read
            event: tool finished (error: no such file)
harness   → model: both results, the second marked as an error
```

## 4. Events

- **Tool started** — call id, tool name, input.
- **Tool finished** — call id, output text, whether it is an error.

A UI can show what the agent is doing from these two alone.

## 5. Context

The context is what the harness gives a tool besides its input.

- **Working directory** — where relative paths resolve.

It is the place for cancellation and similar things later. A tool that needs
nothing from it ignores it.

## 6. Error handling

- **Unknown tool** — the model named a tool that is not registered. Error
  result naming the tool; nothing runs.
- **Input does not fit** — the input cannot be parsed into the tool's input
  type. Error result with the reason; the tool does not run.
- **Tool returns an error** — error result with the error's message.
- **Tool panics** — error result saying the tool crashed. The session
  continues.
- **Errors say so in their text** — the result's text states that it is an
  error, since not every adapter passes the error flag on to the model.

## Open questions

1. Two tools with the same name: refuse to build, or does the last one win?
2. Should independent calls in one round run at the same time?
3. Output beyond text (images, structured data).
4. A limit on output size, and who truncates.
5. Time limits on a tool call.
6. Cancelling a running tool.
7. Must tools be movable between threads?
8. Tools whose name and input are only known at run time, such as ones
   provided by an MCP server.
