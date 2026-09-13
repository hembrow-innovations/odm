# ODM

Orchestrated Development Management — poly-repo workspace OS for humans and AI agents.

## Language

**Workspace**:
The on-disk root a user works in (or points at with `--root`), containing ODM config and state. Created for consumers by `odm init`; distinct from the ODM product repository.
_Avoid_: monorepo root (when meaning the ODM-managed unit), installation, sandbox

**Project**:
A named, config-declared path inside a Workspace, usually its own git checkout (plain clone by default; opt-in gitlink per entry). Only declared entries are Projects — not every subdirectory.
_Avoid_: submodule (as the ODM entity name), package (when meaning a managed repo), repo (as the ODM entity name)

**Progen**:
A named docs/memory store (Markdown on disk plus a disposable index) that ODM orchestrates at a config-declared path. Not owned by `.odm/`. Often its own git repo so history is tracked; path may nest under a Project without merging the two entities. Not a Project unless also declared as one.
_Avoid_: brain, knowledge base (as the product noun), vault (as the ODM entity name), memory root (except when describing progenitor internals)

**Progen group**:
A named, config-only grouping of Progen names used as a query/context scope. Not a store and not stored inside any Progen.
_Avoid_: combo, bundle, profile (for this concept), federation (as the entity name)

**ODM state directory**:
The Workspace-local `.odm/` tree holding ODM config and runtime state only (config, pin, caches, logs, ODM-side progen indexes). Does not own Project checkouts, Progen stores, or Worktree slots (`worktrees/`). Layout: `docs/reference/architecture.md`.
_Avoid_: workspace root (for this path), progen root, plugin home (legacy Go sense)

**Primary checkout**:
A Project’s main working tree at its config path. Implicit — not a separate config entity.
_Avoid_: main worktree (as a competing formal name), default clone

**Worktree slot**:
A named, ODM-managed git worktree placement for parallel or agent work on a Project — separate from that Project's Primary checkout. On disk at `worktrees/<project>/<slot>/` (not under `.odm/`). Bound to a git branch; branch itself stays plain git vocabulary.
_Avoid_: branch (as the ODM entity), sandbox, clone (for this concept), workspace branch

**Workspace config**:
The sole layout source of truth for a Workspace (canonically `.odm/odm.config.yaml`): Projects, Progens, Progen groups, actions, and related declarations. Lives under the ODM state directory with other ODM config/state — not at Workspace root.
_Avoid_: manifest (alone), settings, odm.json as the primary name, root `odm.config.yaml` (legacy Go)

**Gitlink**:
Opt-in membership for a managed Project or Progen (`checkout: gitlink`). Clones remain the default. Pin authority is the parent index SHA. The pin file does not list gitlink names.
_Avoid_: default membership, submodule (as the ODM entity name)

**Pin file**:
An optional lock of resolved revisions for managed clone checkouts (canonically `.odm/odm.lock.yaml`). Not layout truth. Gitlink names are not listed. Record is `odm pin record`. Apply for clones is explicit detached HEAD.
_Avoid_: lockfile (unless paired with Pin file), listing gitlink names

**Action**:
A named task the CLI can invoke — Nx-task-like, not a built-in ODM verb. Defined in Action bundle files pointed to from Workspace config.
_Avoid_: plugin command (legacy), script (as the entity name), target (unless we later alias Nx users)

**Generator**:
A named scaffold from a template package (path or remote URL). Defined in Generator bundle files pointed to from Workspace config. Not an Action.
_Avoid_: schematic (unless aliasing), cookiecutter (as the entity name)
