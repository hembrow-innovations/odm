# Workspace config

Canonical shape of **Workspace config** for ODM v1 design. Domain terms: root `CONTEXT.md`. Git lifecycle: `docs/reference/multi-git.md`.

## Files

- **`.odm/odm.config.yaml`** — sole layout source of truth; lives under the **ODM state directory** at the Workspace root. YAML only for v1; JSON twin deferred.
- **Action / Generator bundle files** — paths declared from Workspace config (relative to Workspace root); free basenames; may live anywhere in the Workspace (not required under `.odm/`).
- **`.odm/odm.lock.yaml`** — optional **Pin file** beside Workspace config when present. Not referenced from inside config; fixed basename discovery only. Semantics: `multi-git.md`.

Missing Action or Generator maps means none are defined (not an error). A declared bundle path that does not exist is an error.

## Top-level keys (`.odm/odm.config.yaml`)

| Key | Required | Shape |
|-----|----------|--------|
| `name` | no | string — Workspace label (UX/docs) |
| `manage_gitignore` | no | bool — default **true**; when Workspace is a git repo, ODM maintains ignore rules for managed paths (`multi-git.md`) |
| `projects` | no* | map name → Project entry |
| `progens` | no* | map name → Progen entry |
| `progen_groups` | no | map name → list of Progen names |
| `actions` | no | map bundle name → path to Action bundle file |
| `generators` | no | map bundle name → path to Generator bundle file |

\*A useful Workspace usually has at least one Project or Progen; empty config is valid for bootstrap.

All entity collections are **maps keyed by name**, not arrays. Keys use **snake_case**. Project and Progen names are unique across both maps and must be path tokens (no `/`, `\`, `.`, or `..`) — same rules as worktree slot names.

No top-level: layout templates, worktree slots, env profiles, inline Action/Generator bodies. Gitlink is opt-in per entry via `checkout`, not a top-level submodule map.

## Project entry

```yaml
projects:
  api:
    path: apps/api                    # required; Primary checkout; relative to Workspace root
    url: https://github.com/acme/api.git   # optional; when set, entry is git-managed
    branch: main                      # optional; clone checkout preference (not a pin)
    type: service                     # optional metadata string
  nested:
    path: vendor/nested
    url: https://github.com/acme/nested.git
    checkout: gitlink                 # optional; omit for plain clone (default)
```

- **`checkout`**: optional. Default is a plain clone. `gitlink` is opt-in gitlink membership. No other submodule fields.
- Pinned revision is **not** a layout field (optional Pin file + `odm pin record` / `odm pin apply`). Gitlink names are not listed in the pin file.
- Parallel checkouts of the same remote = **separate entries** (different name/path/`branch`), not multiple trees under one name.

## Progen entry

```yaml
progens:
  product:
    path: docs                        # required; store root; may nest under a Project path
  eng:
    path: apps/api/docs
    url: https://github.com/acme/api-docs.git   # optional; when set, git-managed like a Project
    branch: main                      # optional
    checkout: gitlink                 # optional; omit for plain clone (default)
```

- Not a Project unless also listed under `projects`.
- Index/cache locations are engine defaults, not config fields.
- Git lifecycle for `url` entries matches Projects (`multi-git.md`). Optional `checkout: gitlink` is opt-in gitlink, same as Project.

## Progen group

```yaml
progen_groups:
  core:
    - product
    - eng
```

- Values are Progen **names** only (strings).
- Unknown name → config load error.
- Groups are config-only scope aliases; nothing is written into any Progen store.

## Actions (file pointers + bundles)

Workspace config points at bundle files; it does not embed task bodies.

```yaml
# .odm/odm.config.yaml
actions:
  core: actions/core.yaml
  api: apps/api/odm.actions.yaml
```

Each bundle file is a map of **Action name → definition**:

```yaml
# actions/core.yaml
bootstrap:
  tasks:
    - run: pnpm install
      dir: apps/api          # optional; default = Workspace root
```

- Bundle paths are relative to the Workspace root (absolute/`..` rejected).
- **Merge** all bundles into one Action namespace.
- **Duplicate** Action names across bundles → config error.
- Bundle map keys are organizational only (not CLI selectors in v1).
- Empty `actions:` map → no Actions.
- v1 task spine: `run` (shell command string) + optional `dir`. Richer executors (copy/env/plugins, output chaining) deferred.

Actions are Nx-task-like named tasks, not built-in ODM verbs.

## Generators (same pointer pattern)

```yaml
# .odm/odm.config.yaml
generators:
  core: generators/core.yaml
```

```yaml
# generators/core.yaml
package:
  template: ./tooling/generators/package    # path relative to Workspace
adr:
  url: https://github.com/acme/gen-adr.git  # remote template pack (alternative to path template)
```

- Merge Generator names across bundles; duplicates → error.
- Each entry needs a template source: **`template`** (path) and/or **`url`** (remote pack).
- Exact `template.toml` / interactive prompt contract is specified elsewhere; this file only locks config wiring.

## Example (minimal)

```yaml
name: acme-platform
# manage_gitignore: true   # default

projects:
  api:
    path: apps/api
    url: https://github.com/acme/api.git
    branch: main

progens:
  product:
    path: docs

progen_groups:
  default:
    - product

actions:
  core: actions/core.yaml

generators:
  core: generators/core.yaml
```

## Explicitly deferred (not in this spine)

- Layout path templates / macros
- Inline actions or generators inside Workspace config
- Worktree slot declarations
- Env profiles
- `odm.config.json` / root-level `odm.config.yaml` (legacy Go location)
- Legacy Go `documentaton` / plugin fields and a top-level submodule map
- Full generator template package format and Nx shell integration details (`env-generators.md`; local generate v1 landed, remote/templating still deferred)

## Relationship to ODM state directory

Workspace config and the optional Pin file live **under** `.odm/` alongside runtime state. Layout truth is still the config file, not ad-hoc state. Project checkouts and Progen stores stay **outside** `.odm/` at their declared `path`s. Full `.odm/` tree, tracked vs ignored paths, worktrees placement, and Workspace root discovery: `docs/reference/architecture.md`.
