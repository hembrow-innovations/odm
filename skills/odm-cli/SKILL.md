---
name: odm-cli
description: >
  Run and reason about the odm CLI (Orchestrated Development Management) in any
  Workspace — init, status, doctor, sync, pin, project, worktree, progen, find,
  context, run, generate. Use when the user mentions
  odm, Workspace, Progen, pin apply, worktree slots, opt-in gitlink, or
  `.odm/odm.config.yaml`.
allowed-tools: Bash(odm:*)
---

# odm CLI

Agent operator guide for **`odm`** — poly-repo workspace OS for humans and AI
agents. One config (`.odm/odm.config.yaml`), one binary, orchestrated
**Projects** + **Progens** (clones by default, opt-in gitlink, no MCP server).

**Docs:** https://hembrow-innovations.github.io/odm-web/ — Gitlink and `pin record` are Unreleased in this tree. PATH `odm` `0.1.1` may lack them.

## Agent defaults

1. Prefer **`--json`** on every command you parse.
2. Start every session with orientation:

```bash
odm status --json
odm doctor --json
odm pin status --json
```

3. Scope flags take **config names**, never filesystem paths:
   `--project`, `--progen`, `--progen-group`, `--wt`.
4. Discovery: walk up from cwd for `.odm/odm.config.yaml` (stops at `$HOME` /
   FS root). Or pin with `--root <path>` (exact; **no** upward walk).
5. On failure: `odm <cmd> --help` from the binary you are actually running, then `status` / `doctor`, then site docs. If help has no `--gitlink` / `pin record`, you are on `0.1.1`; use a binary built from this repository.

## Domain (mini glossary)

- **Workspace** — root with `.odm/odm.config.yaml` (+ runtime under `.odm/`).
- **Project** — named, config-declared path; usually its own git checkout (plain clone by default; opt-in gitlink per entry).
- **Progen** — named Markdown docs/memory store at a declared path (Obsidian-compatible).
- **Progen group** — config-only list of Progen names for query scope.
- **Primary checkout** — Project main tree at its config `path`.
- **Worktree slot** — named parallel git worktree at `worktrees/<project>/<slot>/`.
- **Gitlink**: opt-in membership (`checkout: gitlink`), not default, not a parallel-work primitive. Pin authority is the parent index SHA. The pin file does not list gitlink names. Isolation on one Project is worktree slots.
- **Pin file** — `.odm/odm.lock.yaml` locked SHAs for managed **clone** entries. Auto-maintained. Gitlink names are not listed. `odm pin record` stages gitlink child HEAD into the parent index only.
- **Action** — named task from action bundles; only via `odm run`.
- **Generator** — local template scaffold; via `odm generate`.

Full glossary: [references/glossary.md](references/glossary.md)

## Quick start

```bash
# bootstrap (cwd becomes Workspace)
odm init
# or: odm init ./my-ws --no-git

odm project add api --path apps/api --url https://github.com/acme/api.git
odm project add nested --path vendor/nested --url https://github.com/acme/nested.git --gitlink
odm progen add docs --path docs
odm sync
odm pin record
odm pin status --json
odm status --json
odm doctor --json
```

## Global flags

```bash
odm --root <path> …           # Workspace root (must contain config); no walk-up
odm --json …                  # machine JSON on stdout; humans on stderr
odm --project <name> …        # Project by config name
odm --progen <name> …         # Progen by name (repeatable; union)
odm --progen-group <name> …   # expand group (repeatable)
odm --wt <slot> …             # worktree slot (needs Project context)
odm -h | --help
odm -V | --version
```

JSON error envelope:

```json
{ "ok": false, "error": { "code": "usage|workspace|operation|not_found", "message": "…", "detail": null } }
```

## Exit codes

| Code | Meaning |
|------|---------|
| **0** | success (`find` empty list still 0) |
| **1** | usage (bad flags, unknown name, clap parse) |
| **2** | Workspace / config |
| **3** | operation failed (git, pin, dirty without force, …) |
| **4** | not found (note id, missing `--wt` slot, …) |

**Passthrough** (may be outside 0–4) after successful spawn:

- `odm run <action>` → action exit
- `odm project git …` → git exit

## Commands

### Orientation

```bash
odm status --json
odm doctor --json
odm doctor --fix --json          # mechanical ODM repairs only
odm pin status --json
odm pin status api --json
odm project list --json
odm progen list --json
```

### Bootstrap

```bash
odm init                          # cwd; git init by default
odm init ./ws --no-git
odm init --json                   # { "root", "git" }
# refuses if already a Workspace
# --interactive / -i → not implemented (exit 1)
```

### Sync and pins

```bash
odm sync                          # all managed (url) entries: materialize + fetch ONLY
odm sync api docs                 # named entities only
odm pin record                    # gitlink only: stage child HEAD into parent index (does not commit)
odm pin record nested --force     # dirty child; named clone → usage 1
odm pin apply                     # clones from lock file + gitlinks from parent index; detached HEAD
odm pin apply api --force         # dirty trees need --force
odm pin status --json
```

**Hard rule:** `sync` never checkout/reset/merge. Clone pins auto-maintain in
the pin file. `pin record` is gitlink-only. `pin apply` restores detached HEAD.
`in_sync` = SHA match only (not “on a branch”). Clones remain the default.
Gitlink is opt-in (`--gitlink` / `checkout: gitlink`). `--gitlink` requires `--url`.

### Projects

```bash
odm project list --json
odm project info api --json
odm project add api --path apps/api --url <git-url> [--branch main] [--type service]
odm project add nested --path vendor/nested --url <git-url> --gitlink  # requires --url
odm project add local --path apps/local --no-clone   # config only
odm project rm api                    # un-declare; tree kept
odm project rm api --delete           # remove tree if clean
odm project rm api --delete --force   # dirty OK
odm project git api -- status
odm project git api -- rev-parse HEAD
odm project git api --wt feat -- status
```

No `project sync` — use top-level `odm sync [name]`.

### Worktree slots

```bash
odm project worktree list api --json
odm project worktree add api feat --branch odm-feat
odm project worktree rm api feat [--force]
odm project worktree prune api [--force]
odm project worktree prune --all [--force]

# bind commands to a slot (must already exist — never auto-created)
odm --project api --wt feat project git api -- status
odm --project api --wt feat run test
```

Missing slot → exit **4**. Prefer `--branch` on `worktree add` when Primary
already has the default branch checked out. Parallel feature work uses slots on
a **clone** Primary. Do not use gitlink for isolation.

### Progens (docs / memory)

```bash
odm progen list --json
odm progen info notes --json
odm progen add notes --path progens/notes
odm progen add eng --path apps/api/docs --url <url>
odm progen add nested-docs --path vendor/docs --url <url> --gitlink
odm progen rm notes [--delete] [--force]

# store façade (single-root — pass --progen when multiple configured)
odm progen reindex
odm progen reindex --progen notes
odm progen doctor --progen notes
odm progen ls --progen notes --json
odm progen tree --progen notes
odm progen get welcome --progen notes --json
odm progen body welcome --progen notes
odm progen backlinks readme --progen notes
```

### Find and context

```bash
odm find Token [--limit 5] --json
odm find --progen notes                 # empty query = list scoped notes
odm find foo --progen-group core
odm context welcome --progen notes --json
odm context notes:welcome --json
```

Find is **FTS5 whole tokens** (whitespace = AND), not substring/prefix.
CamelCase is one token. Default `--limit` 200 per store; `0` rejected.

### Actions and generators

```bash
odm run --json                          # list
odm run hello
odm run hello --project api
odm --project api --wt feat run test
odm run build -- -- --flag              # extra args after --
odm --json run hello                    # { action, exitCode, stdout, stderr }

odm generate --json                     # list
odm generate hello --dest out/hello --dry-run
odm generate hello --dest out/hello
odm generate hello --dest out/hello --force   # non-empty dest
```

Actions are **only** via `odm run` (never top-level verbs). Url-only
generators list OK; run is deferred (exit 1).

## Hard rules

1. **Config is sole layout truth** — never invent undeclared Projects/Progens.
2. **`sync` ≠ checkout** — materialize + fetch only; `pin apply` for detached HEAD.
3. **`in_sync` = SHA match only**.
4. **`--wt` never auto-creates** (missing → 4).
5. **Names not paths** on scope flags.
6. **Managed** = entry has `url`; path-only skips git lifecycle. Gitlink only when `checkout: gitlink` (clones remain the default). `--gitlink` requires `--url`.
7. **No `serve` / MCP / daemon** — not shipped. No `odm submodule`. No `odm pr`. Gitlink is never the default.
8. **No path-valued scope flags** and no top-level Action names as CLI verbs.
9. Prefer **`--json`**; re-orient with `status` / `doctor` after failures.
10. When unsure shipped vs deferred, trust **this tree’s** `odm <cmd> --help` (build from this repo if PATH is `0.1.1`).

## Decision tree

```text
Need layout / health?     → status, doctor, pin status, project|progen list|info
Bootstrap empty dir?      → init
Add clone repos?          → project add / progen add → sync → pin status (clone pins auto-maintain)
Add opt-in gitlink?       → project add / progen add --gitlink --url … → sync → pin record
Restore SHAs?             → pin apply [--force] (clones: lock file; gitlink: parent index)
Parallel feature branch?  → worktree add --branch → --wt <slot> on git|run (clone Primary)
Search notes?             → find (federated) / context (one note)
Read note body?           → progen get|body|ls|tree|backlinks
Run workspace task?       → run <action>
Scaffold files?           → generate <name> --dest …
```

## Example workflows

### Orient in an existing Workspace

```bash
odm status --json
odm doctor --json
odm project list --json
odm progen list --json
odm find "architecture" --limit 10 --json
```

### Parallel feature work on a Project

Use a clone Project. Gitlink is one parent-index SHA and is the wrong isolation tool. Push and PR stay with git / the forge.

```bash
odm project worktree add api feat-x --branch feat/x
odm --project api --wt feat-x project git api -- status
odm --project api --wt feat-x run test
odm project worktree rm api feat-x
```

### Opt-in gitlink (not default)

`--gitlink` requires `--url`. `pin record` does not commit the workspace root.

```bash
odm project add nested --path vendor/nested --url https://github.com/acme/nested.git --gitlink
odm sync nested
odm pin record nested
# commit the parent index yourself
odm pin apply nested
odm doctor --json
```

### Knowledge lookup for an agent task

```bash
odm find "worktree slot" --limit 5 --json
odm context notes:welcome --json
odm progen body welcome --progen notes
```

## Not shipped (do not invent)

- `odm serve`, MCP, long-running agent sessions
- `odm submodule` (opt-in gitlink is `--gitlink` / `checkout: gitlink`)
- Gitlink as the default membership
- `odm pr` / forge / PR bot
- `init --interactive`
- Remote generators / template vars / prompts
- Runtime matrix (no default claude/cursor binary)
- Reserved progen verbs: `refs`, `task`, `archive`, `watch`, …
- Path-valued `--project` / `--root-path` legacy flags
- Top-level Action names as CLI subcommands

## Specific tasks

- **Domain glossary** — [references/glossary.md](references/glossary.md)
- **Workspace config** — [references/workspace-config.md](references/workspace-config.md)
- **Projects, git, worktrees** — [references/projects-git-worktrees.md](references/projects-git-worktrees.md)
- **Progens, find, context** — [references/progens-find-context.md](references/progens-find-context.md)
- **Actions and generate** — [references/actions-generate.md](references/actions-generate.md)
- **Troubleshooting** — [references/troubleshooting.md](references/troubleshooting.md)
