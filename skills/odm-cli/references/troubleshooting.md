# Troubleshooting

## First response

```bash
odm --version
which odm
odm status --json
odm doctor --json
odm pin status --json
odm <failing-command> --help
```

Prefer `--json` so you can read `error.code` / `error.message` / `error.detail`.

## Exit code map

| Code | `error.code` | Typical causes |
|------|--------------|----------------|
| 0 | — | Success; `find` with zero hits |
| 1 | `usage` | Bad flags, unknown entity/action/generator, conflicting `--wt`, `init -i` |
| 2 | `workspace` | Not a Workspace, invalid YAML/config, missing bundle path, bad pin file |
| 3 | `operation` | Git/pin/store/generate failure; dirty tree without `--force`; non-empty generate dest; prune left nonempty orphans |
| 4 | `not_found` | Unknown note id; missing worktree slot; missing pin file on apply |

Passthrough after spawn: `run`, `project git` may exit outside 0–4.

## Common failures

### `not a Workspace` / exit 2

- Cwd is not under a tree with `.odm/odm.config.yaml`.
- Fix: `cd` into Workspace, or `odm --root /path/to/ws …`, or `odm init`.

### Unknown project / progen name / exit 1

- Scope flags take **config names**, not paths.
- Check: `odm project list --json` / `odm progen list --json`.

### `--wt` missing / exit 4

- Slot was never created, or name typo.
- Fix: `odm project worktree list <project> --json` then `worktree add`.

### `sync` left me on wrong commit

- Expected: sync is **fetch only**.
- Fix: `odm pin apply` (detached HEAD at pin) or `odm project git <name> -- checkout <ref>`.

### Dirty tree blocks pin apply / delete / pin record

- Use `--force` only when intentional.
- Or clean/stash via `odm project git <name> -- status` first.
- `odm pin record` on a named **clone** is usage 1. Record is gitlink-only.

### `--gitlink` without `--url` / exit 1

- `--gitlink` requires `--url`.

### `odm pin record` / `--gitlink` unknown on PATH

- Released `odm 0.1.1` lacks gitlink. Build from this repository.

### Doctor gitlink fails

- Codes: `gitlink_extra`, `gitlink_missing`, `gitlink_conflict`, `checkout_mismatch`, `gitmodules_layout`.
- `--fix` may rewrite `.gitmodules` and gitignore from config. It does not import extra gitlinks, rewrite remotes, or pin apply.

### `find` returns nothing but notes exist

- Rebuild index: `odm progen reindex`.
- Remember FTS is **whole tokens**, not substring/prefix.
- Try exact tokens from the note body; split CamelCase mentally into real words if notes use spaces.

### Multiple Progens — get/body/context fails

- Pass `--progen <name>` or use `name:id` for context.

### `generate` refuses non-empty dest

- Preview: `--dry-run`.
- Overwrite: `--force`.

### `init` refuses

- Already a Workspace (config present) — no silent overwrite.
- Use existing root or choose a new path.

### `init --interactive` fails

- Reserved, not implemented → exit 1. Use headless `odm init`.

### Binary not found

- Confirm `odm --version` / `which odm`. Put the binary on `PATH` if needed.

## Doctor vs progen doctor

| Command | Scope |
|---------|--------|
| `odm doctor` | ODM-side: config, gitignore, pin basics, worktree orphan/dirty warns, gitlink occupancy |
| `odm doctor --fix` | Mechanical ODM repairs only — `.gitmodules` / gitignore from config; no destructive git, no orphan delete, no extra-gitlink import |
| `odm progen doctor` | Store-side: vault path + index health |

## When docs disagree with the binary

Trust in this order:

1. `odm <cmd> --help` and `--json` from a binary built from this repository (not a stale PATH `0.1.1`)
2. https://hembrow-innovations.github.io/odm-web/
3. This skill (may lag a release)

## Invented commands checklist

If you were about to run any of these, stop:

- `odm serve` / MCP
- `odm submodule` (opt-in gitlink is `--gitlink` / `checkout: gitlink`)
- treating gitlink as the default membership
- `odm pr` / forge commands
- `odm pin record` on a clone name
- `odm project sync`
- path-valued `--project ./apps/api`
- top-level `odm <action-name>` instead of `odm run <action-name>`
- reserved progen verbs (`watch`, `archive`, `task`, …)
- assuming `--wt` creates slots
