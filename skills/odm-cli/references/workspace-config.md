# Workspace config

## Files

- **`.odm/odm.config.yaml`** — sole layout truth (YAML only in v1)
- **`.odm/odm.lock.yaml`** — optional Pin file (fixed basename; not referenced from config)
- **Action / Generator bundles** — paths declared in config; free basenames anywhere under Workspace root

Missing `actions:` / `generators:` maps means none defined (not an error). A
declared bundle path that does not exist **is** an error (exit 2).

## Discovery

1. If `--root <path>` is set: that path **must** contain `.odm/odm.config.yaml`. No upward walk.
2. Else: walk up from cwd looking for `.odm/odm.config.yaml` (stop at `$HOME` / filesystem root).
3. Empty `.odm/` without config ≠ Workspace.
4. `odm init` is the bootstrap exception (may run with no existing Workspace).

## Top-level keys

```yaml
name: acme-platform                 # optional label
manage_gitignore: true              # default true when Workspace is a git repo

projects:
  api:
    path: apps/api                  # required; relative to Workspace root
    url: https://github.com/acme/api.git   # optional → managed
    branch: main                    # optional clone preference (not a pin)
    type: service                   # optional metadata
  nested:
    path: vendor/nested
    url: https://github.com/acme/nested.git
    checkout: gitlink               # optional; omit for plain clone (default)

progens:
  product:
    path: docs
  eng:
    path: apps/api/docs
    url: https://github.com/acme/api-docs.git
    branch: main

progen_groups:
  core:
    - product
    - eng

actions:
  core: actions/core.yaml           # path → Action bundle

generators:
  core: generators/core.yaml
```

## Name rules

- Project and Progen names are unique **across both maps**.
- Names (and worktree slot names) are path tokens only: no `/`, `\`, `.`, or `..`.
- Entity collections are **maps keyed by name**, not arrays. Keys are **snake_case**.

## Project entry

- `path` required (Primary checkout).
- `url` optional → managed git lifecycle.
- `checkout` optional. Default is a plain clone. `gitlink` is opt-in gitlink membership. No other submodule fields.
- Pin revision is **not** a layout field. Gitlink names are not listed in the pin file.
- Parallel checkouts of the same remote = **separate entries** (different name/path).

## Progen entry

- `path` required (store root; may nest under a Project path).
- Optional `url` / `branch` / `checkout` same as Project.
- Not a Project unless also listed under `projects`.
- Indexes live under `.odm/progen/<name>/` (disposable; rebuild with `progen reindex`).

## On-disk layout

```text
<workspace>/
  .odm/
    odm.config.yaml          # tracked
    odm.lock.yaml            # tracked when present
    cache/  log/  progen/<name>/   # typically gitignored
  worktrees/<project>/<slot>/      # NOT under .odm/; typically gitignored
  # Project / Progen trees only where config declares them
```

## Pin file shape

```yaml
version: 1
pins:
  api:
    rev: "0123456789abcdef0123456789abcdef01234567"  # 40-char lowercase hex
    url: https://github.com/acme/api.git
    branch: main   # optional metadata; rev is authority
```

- Auto-created on first successful managed clone materialize **if** Workspace root is a git repo.
- Auto-maintained after successful clone/sync/git-on-Primary that moves HEAD.
- Gitlink names are not listed. Pin authority for gitlink is the parent index SHA.
- **`odm pin record`** stages gitlink child HEAD into the parent index (gitlink names only; does not commit). Clone pins are auto-maintained, not recorded by that verb.
- Apply is **explicit**: `odm pin apply` (clones from this file; gitlink from the parent index).

## What config does not contain

No top-level: layout templates, worktree slot declarations, env
profiles, inline Action/Generator bodies. Gitlink is opt-in per entry via
`checkout`, not a top-level submodule map.
