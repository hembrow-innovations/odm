# Progens, find, and context

## Lifecycle

```bash
odm progen list --json
odm progen info <name> --json
odm progen add <name> --path <rel> [--url <url>] [--branch <b>] [--gitlink] [--no-clone]
odm progen rm <name> [--delete] [--force]
```

Same add/rm/materialize semantics as Projects. `--gitlink` sets `checkout: gitlink`
(opt-in gitlink) and requires `--url`. Clones remain the default. Entity summary
verb is **`info`** (not `get` — `get` is a note-by-id store verb).

Path-only Progens are valid (local Markdown vaults with no remote).

## Store façade (single-root)

ODM resolves Progen **name → path** via config, then runs store ops.

When multiple Progens are configured, pass **`--progen <name>`** (exactly one
for write/single-root reads). Sole configured Progen → bare commands OK.

```bash
odm progen reindex [--progen <name>]
odm progen doctor [--progen <name>]
odm progen ls [--progen <name>] [--json]
odm progen tree [--progen <name>]
odm progen get <id> [--progen <name>] [--json]
odm progen body <id> [--progen <name>]
odm progen backlinks <id> [--progen <name>]
```

- **`get`**: note metadata / node by id.
- **`body`**: note body only.
- **`ls` / `tree`**: inventory paths.
- **`backlinks`**: notes that wikilink to id.
- **`reindex`**: rebuild disposable index under `.odm/progen/<name>/`.
- **`doctor`**: store-side health (path + index) — distinct from top-level `odm doctor`.

### Not shipped under `odm progen`

Do not invent: `refs`, typed node verbs (`task`/`issue`/…), `archive`, `log`,
`glossary`, `plan`, `watch`, `scan`, `serve`.

## Federated find

```bash
odm find [query] [--limit <n>] [--progen …] [--progen-group …] [--json]
```

- Fans out FTS across selected Progens; merges results (tagged with progen name).
- Default scope: **all** configured Progens. Narrow with `--progen` /
  `--progen-group` (repeatable; union).
- Empty query → list scoped notes.
- Empty `progens` map → error (no scope).
- Zero hits → exit **0**, empty list.
- `--limit`: max hits **per store**, default **200**; `0` rejected (usage 1).

### Query semantics (critical for agents)

- Plain text; each **whitespace token** is an FTS5 **whole token** (AND).
- **Not** substring or prefix search.
- CamelCase is **one** token: `TodoWelcome` does **not** match `TodoWelcomeToken`.
- Prefer full tokens or whitespace-separated words that appear in notes.

```bash
odm find DeskUniqueToken --limit 5 --json
odm find architecture decision --progen-group core --json
odm find --progen notes --json
```

## Context (one-hop neighborhood)

```bash
odm context <id> [--progen <name>] [--json]
odm context <progen-name>:<id> [--json]
```

- **In-store only** — no cross-store graph walk.
- Fixed one-hop as `ContextHit`: `anchor` / `outgoing` / `incoming`.
- No `--depth` or facet flags.
- Disambiguation:
  - Multiple Progens → require `--progen` **or** `name:id` prefix.
  - Sole Progen → bare `id` OK.
  - At most one `--progen`. Conflicting `name:id` vs `--progen` → usage 1.
- Unknown id → exit **4**.

```bash
odm context welcome --progen notes --json
odm context notes:welcome --json
```

## Agent workflow for knowledge tasks

```bash
# 1. orient stores
odm progen list --json

# 2. ensure index fresh if finds look stale
odm progen reindex

# 3. search
odm find "your tokens" --limit 10 --json

# 4. pull neighborhood + body for a hit
odm context notes:some-id --json
odm progen body some-id --progen notes
```
