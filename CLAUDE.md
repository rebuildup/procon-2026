# CLAUDE.md

This file is intentionally short. Claude Code should follow:

1. **`AGENTS.md`** — project identity, SoT, active release rule, decision precedence, validation entry point.
2. **`.claude/skills/*/SKILL.md`** — load only the Skill(s) relevant to the current objective. Do not reload the whole skill set for every task.
3. **`docs/`** — architecture, development, release, security, recovery, troubleshooting, ADR.

Do not paste long policy text into this file. If a section needs to change, edit the canonical source under `docs/` or `.claude/skills/`.

## Claude Code specific notes

- **Tooling**: prefer `rg` / `rg --files` for search. Use `just` for the validation gate (see `justfile`). Use `bun` for JS/TS.
- **Worktree**: implementation workers must use isolated sandboxes, not just a Git worktree. Worktree alone does not isolate port / process / database state.
- **Issue / PR workflow**: every non-trivial task gets a GitHub Issue, a ticket branch named with the Issue number, and a Draft PR to the active release branch. PR title / body / reviews are Japanese.
- **Commits**: English. Format `<scope>: <terse imperative summary>`.
- **Skills**: 8 project-local Skills are installed under `.claude/skills/`. Update them via `bunx skills update` (project scope).
- **Recovery**: if context becomes long or session is interrupted, write a structured checkpoint to the durable state (Git commit / Issue comment / PR comment) before exiting. A fresh agent must be able to resume from durable state without prior conversation.