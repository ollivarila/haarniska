# Technical debt log

- Split into separate crates e.g. tui, harness, tools, hooks, providers (might not be fully worth for all)
- Split things behind features
- Move the Anthropic adapter onto the Bedrock adapter's Messages API translation and drop genai (two translations today, see ADR 0004)
