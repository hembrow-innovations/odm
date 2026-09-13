# Vision

ODM is a poly-repo workspace OS for humans and AI agents: one config, one CLI, and orchestrated Projects + Progens (clones by default, opt-in gitlink) without a second brain product.

Domain terms: root `CONTEXT.md`. System shape: `architecture.md`.

## Who it is for

Humans and agents are equal primary users of the same **Workspace**.

- **Shared job** — one desk for code, memory, and agent work, declared in config.
- **Humans** — CLI, Workspace config, sync/pin/status/doctor, actions.
- **Agents** — worktree slots and progen context on that same desk.

Not an AI-first tool that humans can also use.

## Core jobs

- **Many checkouts, one desk** — declare Projects and Progens in config; plain clones by default, opt-in gitlink, and pins.
- **Memory is multi-store** — orchestrate several Progens (scope and federation) without pretending one vault is the world.
- **Agents share the desk** — worktree slots so agent work lands in known places, not ad-hoc clones.
- **One binary UX** — humans and agents do not learn a separate progen CLI or a plugin zoo.

## Instead of…

- **Gitlink as the default desk**: clones remain default; gitlink is opt-in per entry (`checkout: gitlink`); layout lives in Workspace config.
- **One mega-vault** — many Progens, ODM-scoped query; no cross-store wikilinks inside a store.
- **Ad-hoc agent clones** — declared Projects and worktree slots on one desk.

## Non-goals

- Not a monorepo build system (does not replace pnpm, Nx, Cargo workspaces, or similar inside Projects).
- Not a git host, forge, or PR bot.
- Not a second knowledge product — Progen is the store engine; ODM orchestrates.
- No `serve` / MCP daemon in the v1 design package.
- Gitlink as the default membership (clones remain default; gitlink is opt-in per entry).
- The ODM product repository is not a consumer Workspace.

## Ownership (summary)

- **ODM** — Workspace, config, pin, multi-git lifecycle, progen federation/scope, CLI UX, action/generator dispatch, `.odm/` and `worktrees/` placement.
- **Progen (crates)** — single-store content, index, query/context internals; store verbs under `odm progen …`.
- **Shell-out** — `git`; action command bodies.
- **User** — auth, commit policy, content of Projects and Progens.

Full boundary table and crate intent: `architecture.md`.

## Product shape

- Distributed as a **static `odm` binary** (packaging channels beyond the design package are unspecified here).
- **ODM product repository ≠ consumer Workspace**; `odm init` bootstraps the latter.
- Implementation home is this **Rust monorepo**; legacy Go is archive/inspiration only (delivery: `phased-delivery.md`; research: `research/legacy-go-odm.md`).

## Related

- Architecture (state dir, ownership, crates): `architecture.md`
- Config: `config.md`
- CLI: `cli.md`
- Progen federation: `progen.md`
- Multi-git + pins: `multi-git.md`
- Phased delivery (greenfield, phases, ship): `phased-delivery.md`
