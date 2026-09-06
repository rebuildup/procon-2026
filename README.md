# procon-2026

全国高専プロコン 競技部門の monorepo。Rust の core / engine / protocol / cli と、 SolidJS + WebGPU の viewer / UI を1つの repository で統合管理する。

> **Status**: 初期化済み。ドメインルール・問題文は GitHub Issue と ADR で順次追加。

## ディレクトリ構成

```
.
├─ AGENTS.md              ← dispatcher (Claude / Codex / 他 tool の入口)
├─ CLAUDE.md              ← Claude Code 用の薄い pointer
├─ Cargo.toml             ← Rust workspace (edition 2024)
├─ crates/
│   ├─ domain/            ← 純粋ドメイン型・不変条件・ゲームルール (no I/O)
│   ├─ protocol/          ← Rust ↔ Frontend 間の JSON v1 メッセージ
│   ├─ engine/            ← 試合状態機械・replay・match table
│   └─ cli/               ← procon binary (hello / replay / version)
├─ packages/
│   └─ frontend/          ← SolidJS + Vite + WebGPU
├─ docs/
│   ├─ architecture.md
│   ├─ development.md
│   ├─ release.md
│   ├─ security.md
│   ├─ recovery.md
│   ├─ troubleshooting.md
│   ├─ quality.md
│   ├─ adr/               ← 重要な設計判断
│   └─ roles/
├─ .claude/skills/        ← 8 つの project-local Agent Skills
├─ .github/
│   ├─ workflows/ci.yml
│   ├─ ISSUE_TEMPLATE/
│   ├─ PULL_REQUEST_TEMPLATE.md
│   ├─ CODEOWNERS
│   ├─ dependabot.yml
│   └─ release-drafter.yml
├─ justfile               ← deterministic validation entry point
├─ Containerfile          ← 任意の sandbox runtime 用 image 定義
└─ skills-lock.json       ← `bunx skills` 再現性のための lockfile
```

## クイックスタート

```bash
# 1. 前提
rustup toolchain install stable   # 1.85+ (edition 2024)
# bun / node 22+ / gh CLI

# 2. 依存
cargo build --workspace
(cd packages/frontend && bun install)

# 3. 検証 (just がある場合)
just --list
just check          # fmt + clippy + lint + typecheck
just test           # unit
just ci             # ローカル CI と同じ

# 4. CLI 動作確認
cargo run -q --bin procon -- hello
cargo run -q --bin procon -- version

# 5. フロントエンド起動
(cd packages/frontend && bun run dev)
```

詳細は `docs/development.md` を参照。

## 開発フロー

- Issue 駆動: 1 ticket = 1 Issue。branch 名は Issue 番号のみ。
- Release sprint: `release-x-y-z` を active integration line にする。
- Draft PR → Ready PR → release PR の順で品質 gate を満たす。
- Issue / PR / docs は日本語、source code / commit は英語。

## AI エージェント並行駆動

この repository は **複数 AI エージェントの並行作業**を前提に設計されている:

- `.claude/skills/parallel-orchestration/` — task graph 分割と immutable snapshot / result の取り決め
- `.claude/skills/sandbox-runtime/` — 1 worker = 1 isolated runtime
- `.claude/skills/agent-recovery/` — context loss / sandbox 損失からの復旧
- `.claude/skills/quality-gate/` — stack-aware quality profile
- `.claude/skills/engineering-decisions/` — decision precedence と escalation policy
- `.claude/skills/github-delivery/` — Issue / Project / PR / release branch workflow
- `.claude/skills/security-maintenance/` — framework / dependency advisory intake
- `.claude/skills/onboarding/` — fresh contributor / agent onboarding docs

通常 task では必要な Skill だけをロードすればよい (full policy は AGENTS.md → docs/ → Skills と progressive disclosure)。

## ライセンス

MIT OR Apache-2.0 (dual)。