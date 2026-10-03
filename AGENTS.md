# haarniska

A coding agent harness in Rust, shipped as a library.

## Design docs are the source of truth

Read `docs/` before building: `prd.md`, `spec/`, `adr/`. Follow the
`write-docs` skill when writing them.

## Development

Build and tests must pass after making changes.

Define error types with `thiserror`. Do not hand-write `Display` or
`std::error::Error` impls for them.

Always write public API first. Module should be read top down. Constants still go first.

## Commits and changelog

- Use [Conventional Commits](https://www.conventionalcommits.org/):
  `<type>(<scope>): <summary>`, e.g. `feat(core): add session store`. Types:
  `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`.
  Breaking changes use `!` (`feat(core)!: ...`).
- `CHANGELOG.md` is generated from commit messages at release time. Do not
  edit it by hand. Write `feat` and `fix` summaries so they make sense to a
  user reading the changelog.
