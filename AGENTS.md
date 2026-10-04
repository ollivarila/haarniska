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
  user reading the changelog. Only `feat`, `fix` and `perf` commits are listed.
- Releases are cut by [release-plz](https://release-plz.dev). Every push to
  `main` updates a release PR with the version bump and changelog. Merging
  that PR publishes to crates.io, tags `vX.Y.Z` and creates a GitHub release.
  Do not bump the version in `Cargo.toml` by hand.
- Versions stay `0.x` until the API is stable: `feat` and breaking changes
  bump the minor version, `fix` bumps the patch. Going to `1.0.0` is a manual
  decision.
