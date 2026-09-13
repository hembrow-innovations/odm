# ODM (Orchestrated Development Management)

Poly-repo workspace OS for humans and AI agents: one config, one CLI, and orchestrated Projects + Progens without submodules or a second brain product.

**Website:** [hembrow-innovations.github.io/odm-web](https://hembrow-innovations.github.io/odm-web/) (source: [hembrow-innovations/odm-web](https://github.com/hembrow-innovations/odm-web))

**Status:** **v0.1.1** — multi-platform GitHub Releases + curl install; spine (multi-git, Progen, Actions) plus **worktree slots** (add/list/rm/prune; doctor orphan/dirty warns) and local **`odm generate`**.

## Install

Requirements: **git** on `PATH`. Actions (`odm run`) need a Unix shell. Prebuilt install needs `curl` + `tar` (macOS / Linux).

### Quick install (primary)

```bash
curl -fsSL https://raw.githubusercontent.com/hembrow-innovations/odm/main/scripts/install.sh | sh
```

Installs to `~/.local/bin` by default (put it on your `PATH`). The script pulls from [GitHub Releases](https://github.com/hembrow-innovations/odm/releases), verifies **SHA256**, and supports `ODM_VERSION=` / `ODM_INSTALL_DIR=`. Four host triples: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`.

Direct tarball download: [Releases](https://github.com/hembrow-innovations/odm/releases) (latest / [`v0.1.1`](https://github.com/hembrow-innovations/odm/releases/tag/v0.1.1)). Windows is not a primary channel; macOS binaries are **unsigned** (no Homebrew / notarization in v1).

See [docs/reference/install.md](docs/reference/install.md) for options, honesty notes, and troubleshooting.

### Build from source (contributors)

```bash
git clone https://github.com/hembrow-innovations/odm.git
cd odm
cargo build -p odm --release
# binary: target/release/odm

# or install into cargo's bin dir:
cargo install --path crates/odm
odm --version
```

Local packaging: `./scripts/release-build.sh`. Local coverage (optional): `./scripts/coverage.sh` (needs `cargo-llvm-cov`).

## Quickstart

```bash
odm init
odm project add alpha --path projects/alpha --url <git-url>
odm sync
odm pin status
odm status
odm doctor
```

Progen (docs/memory stores) and Actions:

```bash
odm progen list
odm find <query>                 # default --limit 200
odm find <query> --limit 5
odm context <id>
odm run            # list actions
odm run <name>
```

`odm status` and `odm project info` report registered worktree slots and orphan slot dirs.

Generators (local template) and worktree slots:

```bash
odm generate                              # list Generators
odm generate <name> --dest <rel-path> [--dry-run] [--force]  # materialize local template (or preview)
odm project worktree list <project>
odm project worktree add <project> <slot> [--branch <b>]
odm project worktree rm <project> <slot>
odm project worktree prune <project> [--force]
odm project worktree prune --all [--force]
```

See [docs/reference/cli.md](docs/reference/cli.md) for full surfaces, [examples/core-desk/README.md](examples/core-desk/README.md) for offline dogfood, and [examples/todo/README.md](examples/todo/README.md) for real-GitHub dogfood + [REVIEW.md](examples/todo/REVIEW.md).

Dogfood Workspace (offline fixtures):

```bash
cargo build -p odm
# full tour: ODM=target/debug/odm examples/core-desk/scripts/dogfood.sh
cd examples/core-desk
# see examples/core-desk/README.md
odm --root . sync
odm progen reindex
odm find DeskUniqueToken
odm run hello
odm generate                              # sample generators/ + hello template
odm generate hello --dest out/hello
```

Network dogfood (real public repos; read-only on remotes):

```bash
cargo build -p odm
ODM=target/debug/odm examples/todo/scripts/dogfood.sh
# TEMP=1 ODM=target/debug/odm examples/todo/scripts/probe.sh
```

## Docs

- **Website** (guides + quickstart): https://hembrow-innovations.github.io/odm-web/
- **Install**: [docs/reference/install.md](docs/reference/install.md) · [site install](https://hembrow-innovations.github.io/odm-web/install.html)
- **Vision**: [docs/reference/vision.md](docs/reference/vision.md)
- **CLI**: [docs/reference/cli.md](docs/reference/cli.md)
- **Architecture**: [docs/reference/architecture.md](docs/reference/architecture.md)
- **Config**: [docs/reference/config.md](docs/reference/config.md)
- **Multi-git**: [docs/reference/multi-git.md](docs/reference/multi-git.md)
- **Progen**: [docs/reference/progen.md](docs/reference/progen.md)
- **Worktrees**: [docs/reference/worktrees.md](docs/reference/worktrees.md)
- **Env / generate**: [docs/reference/env-generators.md](docs/reference/env-generators.md)
- **Graph** (sketch): [docs/reference/graph.md](docs/reference/graph.md)
- **Phased delivery**: [docs/reference/phased-delivery.md](docs/reference/phased-delivery.md)
- **Changelog**: [CHANGELOG.md](CHANGELOG.md)
- **Domain terms**: [CONTEXT.md](CONTEXT.md)

## Development

```bash
cargo test
```

## Legacy Go

The previous Go CLI was removed. Recoverable as git tag `legacy-go-archive` (or history before that change). Not a compatibility baseline — see [docs/reference/research/legacy-go-odm.md](docs/reference/research/legacy-go-odm.md).

## License

MIT
