# Install

How to get the `odm` binary. Product overview: root `README.md`. Version history: root `CHANGELOG.md`.

Visitor-facing install guide (same steps): project site [`install.html`](https://hembrow-innovations.github.io/odm-web/install.html) (source: [hembrow-innovations/odm-web](https://github.com/hembrow-innovations/odm-web)).

## Requirements

- **Runtime**: `git` on `PATH`
- **Actions** (`odm run`): Unix shell
- **curl|sh install**: `curl`, `tar`, and a POSIX-ish shell (macOS / Linux)
- **Build from source** (contributors): Rust 1.70+

## Quick install (primary)

One-liner installs a prebuilt binary from [GitHub Releases](https://github.com/hembrow-innovations/odm/releases) via the canonical script on `main`:

```bash
curl -fsSL https://raw.githubusercontent.com/hembrow-innovations/odm/main/scripts/install.sh | sh
```

Default install directory: `~/.local/bin` (ensure it is on your `PATH`). The script verifies the download with **SHA256** sums shipped on the release (no cosign/minisign in v1).

### Options

- **`ODM_VERSION=<tag>`**: install a specific release (e.g. `v0.1.1`) instead of **latest**
- **`ODM_INSTALL_DIR=<dir>`**: install directory (default `~/.local/bin`)

```bash
ODM_VERSION=v0.1.1 curl -fsSL https://raw.githubusercontent.com/hembrow-innovations/odm/main/scripts/install.sh | sh
ODM_INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/hembrow-innovations/odm/main/scripts/install.sh | sh
```

### Supported host triples

Release assets target these four triples only:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

Unsupported OS/arch (including **Windows**) fails clearly — Windows is not a primary channel in v1.

### Script and assets

- **Script path:** `scripts/install.sh` on `main` (raw URL above).
- **Assets:** four-platform tarballs + `SHA256SUMS` on [GitHub Releases](https://github.com/hembrow-innovations/odm/releases) — first multi-platform cut is **`v0.1.1`**; default install uses **latest**.

## GitHub Releases (direct download)

Browse [github.com/hembrow-innovations/odm/releases](https://github.com/hembrow-innovations/odm/releases) (or the [`v0.1.1`](https://github.com/hembrow-innovations/odm/releases/tag/v0.1.1) tag) and download `odm-<version>-<triple>.tar.gz` for your host plus `SHA256SUMS`. Prefer **latest**.

```bash
tar xzf odm-<version>-<triple>.tar.gz
# archive contains: odm, README-release.txt
mkdir -p ~/.local/bin
mv odm ~/.local/bin/   # or another directory on PATH
odm --version
```

Integrity: when using the install script, SHA256 is verified automatically. For manual download, check against the release `SHA256SUMS`.

## Verify

```bash
odm --version
odm init --help
```

If the shell cannot find `odm`, add the install directory to `PATH` and open a new shell.

## Build from source (contributors)

```bash
git clone https://github.com/hembrow-innovations/odm.git
cd odm
cargo build -p odm --release
# → target/release/odm

cargo install --path crates/odm
# → ~/.cargo/bin/odm (typically)
```

### Release tarball locally

```bash
./scripts/release-build.sh
# → dist/odm-<version>-<host-triple>.tar.gz
```

Optional:

- **`ODM_TARGET=<triple>`** — cross/build for a specific triple (`cargo build --target`)
- **`ODM_RELEASE_PUBLISH=1`** — after build, run `gh release create` (requires authenticated `gh`)

## Non-goals (this release)

- Homebrew formula
- crates.io publish of the binary crate
- Signed/notarized macOS binaries (binaries are **unsigned**)
- Windows as a primary supported channel
