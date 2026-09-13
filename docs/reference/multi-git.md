# Multi-git lifecycle and pin file

How ODM materializes and maintains declared Projects and Progens. Clones remain the default. Gitlink is opt-in gitlink (`checkout: gitlink`). Domain terms: root `CONTEXT.md`. Config shape: `docs/reference/config.md`. Exact CLI verbs/flags: `docs/reference/cli.md` (illustrative names here only).

## Non-goals

- **Gitlink as the default membership.** Clones remain default. Gitlink is opt-in per entry.
- **`odm submodule`.** There is no such command. Opt-in gitlink is `checkout: gitlink` and `--gitlink` on project and progen add.
- **Worktree slots** (parallel agent/human trees on one Project) are **v1** elsewhere (`worktrees.md`: implemented + deferred); this doc covers Primary checkouts and multi-repo via separate config entries.
- ODM does not store credentials; auth is whatever `git` already uses (SSH agent, credential helper, etc.).

## Managed vs unmanaged

A config entry is **managed** when it has a **`url`** (Project or Progen).

- **`url` set, no `checkout: gitlink`**: clone / sync (fetch) / pin record / pin apply / optional tree delete on rm
- **`url` set, `checkout: gitlink`**: opt-in gitlink. Same add/rm/sync/pin-record verbs. Not materialized as a clone. Pin file does not list the name.
- **path only (no `url`)**: **never** runs lifecycle git (sync/pin/clone); user owns the tree entirely

Path-only entries may still exist on disk for local-only layouts. Sync and pin ignore them. Status/list/info report `is_git` only when the path is its **own** checkout root (`.git` at that path). Nesting under a git Workspace does not inherit the ancestor.

## Plain clones (default)

Each managed entry without `checkout: gitlink` has a **Primary checkout** that is a normal git working tree at its config `path` (relative to Workspace root).

- Nested managed paths are allowed (e.g. a Progen repo under a Project path): each remains its own git repo.
- Parallel branches of the "same" remote are **multiple entries** (distinct names, paths, optional `branch`), not one entry with many trees.

### Optional `branch`

Managed entries may set **`branch`**. When set, clone checks out that branch (creating local tracking as git normally would). When unset, clone uses the remote's default HEAD.

`branch` is a checkout preference, not a pin. **Pin authority for clones is the recorded commit SHA.**

## Opt-in gitlink

A managed entry is gitlink **only** when `checkout: gitlink` is set. Absent checkout is a plain clone. Clones remain the default.

- Config field: optional `checkout`. The only gitlink value is `gitlink`. No other submodule fields.
- CLI: `--gitlink` on `odm project add` and `odm progen add` sets that field.
- Pin authority for a gitlink entry is the **parent index SHA**. The pin file does **not** list gitlink names.
- `odm pin record` is the named record verb.
- Pin revision is not a layout field.

## Workspace git

- Workspace root **may** be a git repository; ODM does not require it for day-to-day commands once a Workspace exists.
- **`odm init`** creates a git repo at the Workspace root **by default**. Skip with a no-git flag (exact flag in cli.md).
- When the Workspace is a git repo, managed clone checkouts are ordinary nested directories (usually ignored; see gitignore below). The Workspace repo tracks ODM config and optional pin file under `.odm/`. Full `.odm/` contract: `architecture.md`.

## Materialize (clone)

When ODM must ensure a managed entry **without** `checkout: gitlink` exists on disk:

- **missing**: `git clone <url> <path>` (full history; `branch` if set)
- **empty directory**: clone into it
- **git repo, `origin` URL matches config `url` (normalized)**: already materialized; do not re-clone
- **git repo, `origin` mismatch**: **fail** (no silent rewrite of remotes)
- **exists, not a git repo**: **fail** (never delete or adopt user data)

URL must be acceptable to `git clone`. No zip/rsync fallback in v1.

Gitlink entries are not materialized as clones. They use opt-in gitlink membership from add / `checkout: gitlink`.

## Sync

**Sync** means: ensure present, then refresh remotes. It does **not** move HEAD.

1. If missing (or empty dir): materialize (clone for entries without `checkout: gitlink`).
2. If present and valid: `git fetch` (default remote).
3. **Never** checkout, reset, merge, or rebase as part of sync.

Pin apply is a separate operation when clone trees must match locked revisions.

## Add and remove

Semantics (command names in cli.md):

### Add

1. Write the Project or Progen entry into Workspace config (`path`, `url`, optional `branch` / `type` / `checkout`).
2. If `url` is set and checkout is not gitlink: materialize (clone), unless a **no-clone** option defers disk work (config-only declare for offline/edit flows).
3. `--gitlink` on project and progen add sets `checkout: gitlink` (opt-in gitlink). Do not invent an `odm submodule` command.

### Remove

1. Remove the entry from Workspace config (un-declare).
2. Working tree **stays** by default.
3. Optional **delete** flag: remove the working tree only if the tree is clean (or not a git repo). Dirty tree → **fail** unless **force**. Un-declare still allowed when delete is not requested.

## Nested paths and ordering

Managed paths may nest. When operating on **all** managed entries:

- Order by **increasing path depth** (parents before children) for materialize/sync.
- Fail-fast on the first hard error; do not continue as if the batch succeeded.
- Pin auto-updates apply only to clone entries that succeeded in that run (gitlink names are not listed in the pin file).

Single-name operations affect only that entry (still subject to nest rules if clone would require a missing parent path; parent must already exist or be managed and materialized first).

## Gitignore management

Config key **`manage_gitignore`** (boolean, **default true** when omitted): when the Workspace is a git repo, ODM maintains ignore rules so managed checkout paths are not committed into ancestor repos.

- Updates `.gitignore` in the **Workspace root** and in any **ancestor managed checkout** that contains another managed path.
- When `manage_gitignore` is false, ODM does not edit ignore files (user is responsible).
- Exact ignore file format and markers are an implementation detail; behavior is "managed paths stay untracked in parents."

## Pin file

**Path:** `.odm/odm.lock.yaml` (fixed basename beside Workspace config). Not referenced from inside config. Not a layout field.

### Creation

- **Auto-create** on the first successful materialize (clone) of any managed clone entry **only if** the Workspace root is already a git repository.
- Non-git Workspaces never get a pin file from auto-create (explicit pin init may exist later in CLI; not required for this model).
- Until the file exists, there is no pin behavior for clones.

### While present (auto-maintain)

After successful clone, sync, or other lifecycle ops that leave a defined HEAD on a managed **clone** entry, ODM **updates** that entry's recorded revision to the current full commit SHA.

- Auto-maintain keeps the lock **accurate**.
- Auto-maintain does **not** checkout pins during sync.
- Gitlink names are not listed.

### Contents

```yaml
version: 1
pins:
  api:
    rev: "0123456789abcdef0123456789abcdef01234567"
    url: https://github.com/acme/api.git
    branch: main          # optional metadata; rev is authoritative
  product-docs:
    rev: "abcdef0123456789abcdef0123456789abcdef01"
    url: https://github.com/acme/product-docs.git
```

- Keys are **entity names** (Project or Progen) that are managed **clones**.
- Drop pins for names removed from config on the next successful update pass; add pins when a managed clone is first materialized.
- Path-only entities never appear.
- Gitlink names are not listed. Pin authority for gitlink is the parent index SHA.

### Pin record and pin apply

- **`odm pin record`**: named record verb. For clones, pin authority is the recorded commit SHA. For gitlink, pin authority is the parent index SHA (not a lock-file row).
- **`odm pin apply`**: for each clone pin whose path exists, check out **`rev` as detached HEAD**. Gitlink names are not listed in the pin file.

- Dirty working tree → **fail** unless force.
- Detached is intentional (clone pin authority = commit SHA). CLI output states `detached HEAD`; pin **`in_sync`** means HEAD SHA equals pin `rev`, not "checked out on a branch." Re-attach with ordinary `git checkout <branch>` when you want a branch again.
- Missing path → skip or fail per CLI (design default: fail for named apply; for all-apply, fail-fast).
- Does not change config. After a successful apply, auto-maintain leaves `rev` unchanged (HEAD already at pin).

## Batch vs single

Lifecycle ops accept **one entity name** or **all managed entries**. Default workspace-wide sync = all managed, depth-ordered, fail-fast.

## Related

- Workspace config keys and entry fields: `config.md`
- `.odm/` tracked vs ephemeral layout: `architecture.md` / ODM state directory contract
- Worktree slots (v1 + deferred): `worktrees.md`
- CLI verbs: `cli.md`
