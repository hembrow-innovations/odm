# ODM domain glossary

Use these nouns when talking to users and when choosing CLI verbs. Avoid the
listed false friends.

## Workspace

On-disk root the user works in (or points at with `--root`), containing ODM
config and state. Created for consumers by `odm init`. Distinct from the ODM
product repository.

_Avoid:_ monorepo root (when meaning the ODM-managed unit), installation, sandbox

## Project

A named, config-declared path inside a Workspace, usually its own git checkout
(plain clone by default; opt-in gitlink per entry). Only declared entries are
Projects — not every subdirectory.

_Avoid:_ submodule (as the ODM entity name), package (when meaning a managed repo), repo (as the ODM entity name)

## Progen

A named docs/memory store (Markdown on disk plus a disposable index) that ODM
orchestrates at a config-declared path. Not owned by `.odm/`. Often its own
git repo; path may nest under a Project without merging the two entities. Not
a Project unless also declared as one.

_Avoid:_ brain, knowledge base (as the product noun), vault (as the ODM entity name)

## Progen group

A named, config-only grouping of Progen names used as a query/context scope.
Not a store and not stored inside any Progen.

_Avoid:_ combo, bundle, profile (for this concept), federation (as the entity name)

## ODM state directory

The Workspace-local `.odm/` tree: config, pin, caches, logs, ODM-side progen
indexes. Does **not** own Project checkouts, Progen stores, worktree slots
(`worktrees/`), or generator template packages.

_Avoid:_ workspace root (for this path), progen root

## Primary checkout

A Project’s main working tree at its config `path`. Implicit — not a separate
config entity.

## Worktree slot

A named, ODM-managed git worktree for parallel or agent work on a Project —
separate from Primary. On disk at `worktrees/<project>/<slot>/` (not under
`.odm/`). Bound to a git branch; the branch itself stays plain git vocabulary.

_Avoid:_ branch (as the ODM entity), sandbox, clone (for this concept)

## Workspace config

Sole layout source of truth: `.odm/odm.config.yaml`. Declares Projects,
Progens, Progen groups, action bundles, generator bundles.

_Avoid:_ manifest (alone), settings, `odm.json` as the primary name,
root-level `odm.config.yaml` (legacy)

## Gitlink

Opt-in membership for a managed Project or Progen (`checkout: gitlink`).
Clones remain the default. Not an isolation primitive (use worktree slots on
a clone). Pin authority is the parent index SHA. The pin file does not list
gitlink names. CLI: `--gitlink` on project and progen add (requires `--url`).
`odm pin record` stages child HEAD into the parent index and does not commit.
There is no `odm submodule` command.

_Avoid:_ default membership, submodule (as the ODM entity name), worktree slot

## Pin file

Optional lock of resolved revisions for managed **clone** checkouts:
`.odm/odm.lock.yaml`. Not layout truth. Gitlink names are not listed. Created
when the Workspace is a git repo and managed clones succeed; auto-maintained
while present. **`odm pin record`** is gitlink-only (parent index, not a lock
row). **Apply is explicit** (`odm pin apply`): clones from the lock file,
gitlink from the parent index.

_Avoid:_ lockfile (unless paired with “Pin file”), listing gitlink names, using `pin record` on clones

## Action

A named task the CLI can invoke (Nx-task-like), defined in Action bundle files
pointed to from Workspace config. Invoked only via `odm run`.

_Avoid:_ plugin command, script (as the entity name)

## Generator

A named scaffold from a template package (local path; remote URL deferred for
run). Defined in Generator bundle files. Not an Action.

## Managed entry

A Project or Progen with a `url` field. Path-only entries are declared layout
only (no git lifecycle from ODM). Absent `checkout` is a plain clone and
participates in clone/sync/pin. `checkout: gitlink` is opt-in gitlink.
