# Projects, git, and worktree slots

## Lifecycle

```bash
odm project list --json
odm project info <name> --json
odm project add <name> --path <rel> [--url <url>] [--branch <b>] [--type <t>] [--gitlink] [--no-clone]
odm project rm <name> [--delete] [--force]
odm project git <name> [--wt <slot>] -- <git-args…>
```

### add

- Writes Project entry into Workspace config.
- If `url` set and checkout is not gitlink: materializes (clone) unless `--no-clone`.
- `--gitlink` sets `checkout: gitlink` (opt-in gitlink). Clones remain the default.
- `path` is relative to Workspace root.
- `--branch` is clone checkout preference only — **not** a pin.

### rm

- Un-declares from config; **tree kept** by default.
- `--delete` removes tree if clean; dirty → fail unless `--force`.
- Does **not** delete `worktrees/<project>/`.

### git

```bash
odm project git api -- status
odm project git api -- checkout -b feat/x
odm project git api --wt feat -- status
odm project git api -- rev-parse HEAD
```

- Runs `git -C <primary-or-wt> <args…>` (argv after `--`, not a shell string).
- Exit code = git’s.
- On success, if pin file exists and **HEAD changed on Primary**, auto-maintain
  that entity’s pin (not a full `sync`). Pin maintain is **Primary-only**.
- Global `--wt` and command `--wt` must match when both set (differ → usage 1).

### No project sync

Use top-level:

```bash
odm sync              # all managed
odm sync api          # one entity
```

## Sync vs pin

- **`odm sync [name…]`**: materialize missing managed clones + **fetch only**. Never checkout/reset/merge.
- **`odm pin record`**: named record verb. Clones: lock SHA. Gitlink: parent index SHA. The pin file does not list gitlink names.
- **`odm pin status`**: compare pin file SHAs vs current HEAD for clones.
- **`odm pin apply [--force]`**: checkout each clone pin `rev` as **detached HEAD**. Dirty needs `--force`. Gitlink names are not listed.

`in_sync` means **SHA match only**, not “checked out on a branch.”

Managed = has `url`. Path-only Projects skip git lifecycle. Absent `checkout` is a plain clone. `checkout: gitlink` is opt-in gitlink.

## Worktree slots (v1)

Disk path: `worktrees/<project>/<slot>/` (not under `.odm/`).

```bash
odm project worktree list <project> --json
odm project worktree add <project> <slot> [--branch <b>]
odm project worktree rm <project> <slot> [--force]
odm project worktree prune <project> [--force]
odm project worktree prune --all [--force]
```

### Rules

- Primary must already be a git repo.
- Slot names: same path-token rules as Project names.
- **`--wt` never auto-creates.** Missing slot path → exit **4**.
- Prefer `--branch` on `add` when Primary already has the default branch checked out
  (git cannot create a second worktree on the same branch without a new branch name).
- `status` / `info` report `worktree_slots` and `worktree_orphans`.
- Doctor warns on orphans (`worktree orphan`) and dirty registered slots
  (`worktree_dirty:<project>:<slot>`) — observation only; use `prune` / git to fix.
- `prune` removes orphan dirs that are empty (or with `--force` per prune rules).

### Binding `--wt`

Works for:

- `odm project git <name> --wt <slot> -- …`
- `odm --project <name> --wt <slot> run <action>`

```bash
# parallel work on a slot
odm project worktree add api agent-1 --branch agent/1
odm --project api --wt agent-1 project git api -- status
```

## status / doctor fields (projects)

Prefer `--json`.

- Per project: `worktree_slots: [{ name, path, dirty }]`
  - `dirty`: `true` / `false` / `null` (probe failed)
- Per project: `worktree_orphans: [{ name, path }]` (no `dirty`)
- Entity `is_git` / dirty apply only when the path is its **own** git root
  (has `.git` there) — nested path-only trees do **not** inherit an ancestor repo.
