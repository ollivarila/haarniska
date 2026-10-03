# Spec: Skills

**Date:** 2026-10-03
**Status:** Draft
**Owner:** Olli Varila
**PRD:** [PRD](../prd.md)

## Overview

A skill is a written-down way of doing a recurring task: a folder with
instructions and, optionally, files that go with them. The model always knows
which skills exist, and reads a skill's instructions only when a task calls
for it. This spec covers the format, how skills are found, and how one is
loaded. It covers the "Skills" requirement of the [PRD](../prd.md) and builds
on the [tools spec](2026-10-03-tools-design.md) and the
[project instructions spec](2026-10-03-project-instructions-design.md).

## Locked decisions

- **The Agent Skills format** — a folder with a `SKILL.md`: a header with a
  name and a description, then the instructions. Skills written for other
  agents work unchanged.
- **The caller chooses where to look** — directories are passed in as
  layers, like project instructions. The library has no built-in locations.
- **Later layer wins** — two skills with the same name: the one from the
  later layer is used.
- **Listed in the system prompt** — each skill's name and description, and
  nothing more.
- **Loaded through a tool** — the model asks for a skill by name and gets its
  instructions. The tool is added when skills are passed in.
- **The model decides** — there is no way for the user to invoke a skill
  directly.
- **Headers at startup, bodies on demand** — startup reads only what the
  listing needs.
- **A broken skill is skipped and reported** — it never stops startup, and
  never disappears silently.
- **Loaded by the caller, opt-in** — as with project instructions.

## 1. A skill

```
skills/
  write-docs/
    SKILL.md
    template.md
```

```markdown
---
name: write-docs
description: Conventions for design docs. Use when writing a PRD, spec, or ADR.
---

Docs live under `docs/`. Start every doc with the header in `template.md`.
...
```

- **Name** — how the model asks for the skill.
- **Description** — what the skill is for and when to use it. This is all
  the model sees until it loads the skill, so it decides whether the skill
  gets used.
- **Body** — the instructions, in whatever form helps.
- **Other files** — anything the body refers to. They are not loaded with
  the skill; the model reads them with the usual tools when the body says so.
- **Header format** — YAML, read with a YAML library, so multi-line
  descriptions and quoting work as written.

## 2. Finding skills

```rust
let skills = Skills::new()
    .layer_dir("/home/me/.claude/skills")?  // user-wide
    .layer_dir(".claude/skills")?;          // the project
```

- **A layer** — a directory whose subfolders are skills. A subfolder without
  a `SKILL.md` is ignored.
- **Order** — general first. A project's skill replaces a user-wide one of
  the same name.
- **Missing directory** — the layer is empty. Not an error.
- **Paths as given** — a leading `~` is not expanded.
- **Problems** — the skills found, and what went wrong with the ones that
  were skipped, are both available to the caller.

## 3. What the model sees

Example: two skills, after the system prompt and any project instructions.

```
You are a coding agent.

Skills are available for the tasks below. Load one with the skill tool
before doing a task it covers.

- write-docs: Conventions for design docs. Use when writing a PRD, spec, or
  ADR.
- release: Steps for cutting a release.
```

- **Stable** — the list does not change during a session, so it stays in
  the prompt cache.
- **No skills** — nothing is added, and there is no skill tool.

## 4. Loading a skill

```
user      → "write a spec for the cache"
model     → ToolCall(skill, {name: "write-docs"})
harness   → result: the skill's instructions, and the folder they came from
model     → ToolCall(read, {path: ".../write-docs/template.md"})
model     → writes the spec
```

- **Result** — the body of `SKILL.md`, and the path of the skill's folder so
  the model can find the files the body mentions.
- **Read when asked for** — the body is read from disk at that moment, so an
  edit to a skill takes effect on its next use.
- **An ordinary tool call** — it is shown in the UI and passes through hooks
  like any other.
- **More than once** — loading a skill twice returns it twice. The harness
  does not track what is loaded.

## 5. Using it

```rust
let agent = haarniska::builder()
    .with_inference(inference)
    .with_default_tools()
    .with_skills(skills)
    .build();
```

## 6. Error handling

- **No `SKILL.md` in a subfolder** — not a skill; ignored without a report.
- **Header missing, not valid, or without a name or description** — the
  skill is skipped and reported with its path and the reason.
- **Directory cannot be read** — for a reason other than not existing.
  Adding the layer fails and the caller decides.
- **Unknown skill name** — the model asked for a skill that does not exist.
  Error result naming the skills that do.
- **Skill file gone or unreadable when loaded** — error result with the
  reason. The turn continues.

## Open questions

1. A limit on how many skills are listed, or on the size of the listing.
2. Should the user be able to invoke a skill directly?
3. Should the harness remember which skills are loaded, to avoid loading one
   twice?
4. Skills that only apply to some paths or projects.
5. Should the UI show which skills are available?
