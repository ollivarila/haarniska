# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0](https://github.com/ollivarila/haarniska/compare/v0.0.0...v0.1.0) - 2026-10-04

### Added

- *(tui)* show tool calls on one line with their result below
- *(tui)* render replies as markdown with syntax highlighting
- *(tui)* show time to first paint and to ready
- *(tui)* consent prompt, colors, full-width input line
- *(harness)* let hooks ask for the user's consent
- *(harness)* add skills, loaded on demand
- *(harness)* add hooks around tool calls
- *(inference)* keep thinking blocks across tool rounds, on genai 0.7
- *(tui)* scroll the transcript with keys and mouse wheel
- *(harness)* read layered project instructions from AGENTS.md
- *(inference)* cache the prompt on Anthropic
- *(tui)* add terminal UI, load .env in the example
- *(harness)* add Ui interface and session loop
- *(harness)* implement the agent loop
- *(tools)* [**breaking**] add built-in tools, prefix builder methods with with_
- *(harness)* [**breaking**] replace plugins with a tool API
- *(inference)* add Anthropic model constants
- *(inference)* add data model and Anthropic adapter
- *(inference)* stub Anthropic adapter
- stub kitchen sink example
- stub public API modules

## [0.0.0] - 2026-09-27

### Added

- Placeholder release reserving the crate name.
