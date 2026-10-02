# AGENTS.md — Project-local Root Agent Contract

このファイルは **dispatcher** であり、すべての canonical policy を要約しません。
詳細な policy / spec / role 定義は `docs/` と `.claude/skills/` にあります。

## Project identity

- **Name**: procon-2026 — 全国高専プロコン 競技部門 プロジェクト
- **Repository**: <https://github.com/rebuildup/procon-2026> (private)
- **Domain**: Competitive programming contest — game/algorithm contest (Rust core + SolidJS + WebGPU frontend)
- **Language policy**:
  - Source code / commit message: **English**
  - Internal docs / Issues / PRs / reviews: **日本語**

## Source / Work state (canonical SoT)

| State kind | Location |
|---|---|
| Released source state | `main` |
| Active sprint integration | `release-<major>-<minor>-<patch>` |
| Durable work items | GitHub Issues |
| Work-in-progress board | GitHub Projects (Kanban) |
| Ticket review / integration | Pull Requests |
| Project policy / architecture / decisions | `docs/` + `.claude/skills/` |
| Recovery / immutable worker results | durable Git refs + structured checkpoints |
| Transient execution | Supervisor / sandbox (not SoT) |

## Active release rule

- 1 sprint = 1 target semantic version. Sprint start cuts `release-<major>-<minor>-<patch>` from `main`.
- 1 top-level Issue = 1 durable ticket branch named **only with the Issue number**.
- After a meaningful first commit, open a Draft PR from the ticket branch to the active `release-x-y-z` branch.
- The release PR `release-x-y-z -> main` represents release integration.

## Engineering decision precedence

Resolve decisions in this order:

1. `docs/` + `.claude/skills/` (project-local canonical policy / invariant)
2. ADR (`docs/adr/`) and explicit design/spec
3. coherent existing implementation majority
4. current official framework / runtime / SDK guidance
5. established ecosystem convention
6. local best judgment

Same-level conflicts: prefer the more specific and more recent canonical source.
Old migration residue does not override current canonical design.
When a decision is uniquely determined by precedence and is reversible / local, do not escalate to the user.

## User escalation boundary

Escalate only when:

- canonical sources conflict and product semantics change
- acceptance criteria is multi-interpretable and user-visible behavior shifts
- the operation is irreversible / destructive
- public / external API contract is being fixed
- security / privacy / compliance risk acceptance
- meaningful cost increase
- release scope / date change
- explicit design-first approval gate is required

Always prepare options, impact, and a recommended path before asking.

## Environment bootstrap

Quick start for a fresh clone:

```bash
# Toolchains: Rust 1.85+ (edition 2024), Bun (latest), Node 22+, gh CLI 2.50+
rustup toolchain install stable
cargo install just cargo-watch  # optional but recommended
bun install
gh auth login                  # required for Issues / Projects / PR work
```

`docs/development.md` has the full local bootstrap recipe and per-component commands.

## Supervisor / subagent / recovery entry point

- Subagent / parallel work model: `.claude/skills/parallel-orchestration/SKILL.md`
- Sandbox / runtime portability: `.claude/skills/sandbox-runtime/SKILL.md`
- Recovery / interruption handling: `.claude/skills/agent-recovery/SKILL.md`
- Release / Issue / PR workflow: `.claude/skills/github-delivery/SKILL.md`

Always spawn mutable workers in isolated sandboxes; never share mutable DB / cache / queue / process state between workers.
Parent → child uses immutable snapshots; child → parent returns immutable commit/ref + validation results.
Each mutable worker task carries an `execution_generation` token; stale generations must not integrate.

## Validation entry point

Single deterministic validation entry point — `just` (see `justfile`):

```bash
just --list                  # discover available gates
just check                   # fmt + clippy + type-check + lint
just test                    # unit tests
just test-integration        # integration (real component boundaries)
just ci                      # full local CI parity
```

Detailed quality profile: `.claude/skills/quality-gate/SKILL.md`.
Stack-aware quality choices: `docs/quality.md`.

## Skill discovery

Project-local Agent Skills (Japanese descriptions, English body) are under `.claude/skills/`:

| Skill | When to load |
|---|---|
| `parallel-orchestration` | decomposing non-trivial work across multiple AI workers |
| `sandbox-runtime` | creating / validating isolated worker sandboxes |
| `github-delivery` | Issue / Project / PR / release branch flow |
| `quality-gate` | designing verification gates for this stack |
| `engineering-decisions` | resolving design decisions against project precedence |
| `security-maintenance` | tracking framework / dependency advisories |
| `onboarding` | writing / updating onboarding docs |
| `agent-recovery` | recovering work after session / sandbox / context loss |
| `agent-delivery-estimation` | release / roadmap / capacity / throughput forecasting for AI-agent-driven delivery |
| `correctness-assurance` | extracting invariants and selecting minimum-sufficient assurance mechanisms (type / runtime / test / property / formal / review) |
| `design-refinement` | reading relentlessly and asking minimally to turn requirements into an implementable specification |
| `interaction-discipline` | composing active-work messages (handoff / checkpoint / brief) with minimal noise and maximum signal |
| `linear-release-control` | using Linear as a release-level planning / portfolio control plane alongside GitHub Issues (optional) |
| `policy-evaluation` | deciding between deterministic and latent (LLM-as-judge / human) evaluation per policy property |
| `worktree-workflow` | providing per-worker isolated mutable runtime via Worktrunk / git worktree (filesystem / branch / port / db / cache isolation) |
| `writing-discipline` | applying Select → Compose → Reread pipeline to Issue / PR / ADR / doc prose |

Do not re-read the full `project-init` prompt for routine tasks — load only the Skill(s) that match the current objective.

## Quick navigation

- Architecture: `docs/architecture.md`
- Development / run / debug: `docs/development.md`
- Release process: `docs/release.md`
- Security / advisories: `docs/security.md`
- Recovery: `docs/recovery.md`
- ADRs: `docs/adr/`
- Troubleshooting: `docs/troubleshooting.md`
- Roles / responsibilities: `docs/roles/`
## Constitution / operating profile

- 最上位 contract: [`constitution/CONSTITUTION.md`](constitution/CONSTITUTION.md)
- current Operating Model: [`organization/profiles/release-driven-solo.md`](organization/profiles/release-driven-solo.md)
- 既存の project-specific ADR / architecture / release contract は、Constitution と両立する限り generic upstream Practice より具体的な authority として維持する。


## Agent Skills lifecycle

project-init 由来の Agent Skills は **project-local** に管理し、global install を canonical にしない。

- 初回導入 / 全体 reconcile: `bunx skills add rebuildup/project-init --skill '*' --agent claude-code opencode codex -y`
- fresh clone から lock を復元: `bunx skills install`
- 継続更新: `bunx skills update -p -y`
- `skills-lock.json` は `skills` CLI が生成・更新する source/freshness metadata として commit する。手で hash / source entry を捏造しない。
- upstream-managed Skill 本文は手編集しない。project 固有の refinement / override は別の project-local Skill、adapter、ADR、docs に置き、次回 update で上書きされない構造にする。
- update 後は Git diff と applicable quality gate を確認し、upstream 更新を無条件に current project policy とみなさない。

Bun はここでは Agent Skills 管理用の project tooling であり、product runtime / package manager の既存 decision を自動的に置換しない。
