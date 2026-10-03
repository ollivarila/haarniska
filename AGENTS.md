# haarniska

A coding agent harness in Rust, shipped as a library.

## Design docs are the source of truth

Read `docs/` before building: `prd.md`, `spec/`, `adr/`. Follow the
`write-docs` skill when writing them.

## Development

Build and tests must pass after making changes.

## Commits and changelog

- Use [Conventional Commits](https://www.conventionalcommits.org/):
  `<type>(<scope>): <summary>`, e.g. `feat(core): add session store`. Types:
  `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`.
  Breaking changes use `!` (`feat(core)!: ...`).
- User-visible changes get an entry under `## [Unreleased]` in `CHANGELOG.md`
  (Keep a Changelog sections: Added, Changed, Deprecated, Removed, Fixed,
  Security). Released sections are never edited.
