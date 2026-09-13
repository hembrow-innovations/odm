

- Commits as work packages: `<type>(<scope>): <description>` — `feat` | `fix` | `test` | `refactor` | `chore`
- No `Co-Authored-By` lines
- TDD, DRY, YAGNI; prefer one-liner solutions when clear
- No product CI test matrix; GitHub Actions allowed only for a **release publish** workflow (tag `v*` / `workflow_dispatch` builds that upload release assets). Pages live in `hembrow-innovations/odm-web`.
- Worktrees **disabled** — never `git worktree add` or agent isolation worktrees
- No new git branches for agent work; cleanup when done
- File size target ≤1000 LOC, hard limit 1250
- Markdown: never tables — use `- **{text}**: {text}`
## Agent skills



### Issue tracker

Issues live as markdown under `docs/planning/issues/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Roles are frontmatter tags: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`; reject via `status: wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout (`CONTEXT.md` + `docs/adr/` at repo root). See `docs/agents/domain.md`.
