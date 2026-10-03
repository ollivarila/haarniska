# haarniska

Library-first, extensible coding agent harness. Under development.

See `examples` for demo.

Documentation in `docs` is mostly for agents but should be in readable state.

## Architecture

An agent is a harness plus inference. The harness runs the loops and the
tools; a UI is handed over only for an interactive session.

```mermaid
flowchart LR
    user([User])
    api[(Anthropic<br/>Messages API)]

    subgraph agent [Agent]
        harness["Harness<br/>agent loop, session loop"]
        inference["Inference<br/>AnthropicInference, on genai"]
        tools["Tools<br/>read, write, edit, shell, your own"]
    end

    ui["Ui<br/>Tui, on its own render thread"]

    user -- keys --> ui
    ui -- "Input: prompt, cancel, quit" --> harness
    harness -- "Event: text, tool calls, done" --> ui
    harness -- "Request: system, messages, tools" --> inference
    inference -- "Chunk: text deltas, then the reply" --> harness
    harness -- ToolCall --> tools
    tools -- ToolResult --> harness
    inference <-->|HTTPS| api
```

Headless use skips the UI: `agent.prompt(...)` returns the same events as a
stream.

## License

MIT
