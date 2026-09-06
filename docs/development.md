# Development

ローカルで開発・検証・デバッグするための具体的レシピ集。

## 前提 toolchain

- **Rust** ≥ 1.85 (edition 2024 対応)
- **Bun** ≥ 1.4 (Node.js 22+ 代替可)
- **gh** ≥ 2.50 (PR / Issue 操作)
- **just** (任意、ただしローカル CI に揃えるために強く推奨)
- **rg** / **fd** (高速検索、任意だが推奨)

確認:

```bash
rustc --version && cargo --version
bun --version
gh --version
```

## 初回セットアップ

```bash
# 1. clone
git clone git@github.com:rebuildup/procon-2026.git
cd procon-2026

# 2. Rust deps + build
cargo build --workspace

# 3. Frontend deps
(cd packages/frontend && bun install)

# 4. CLI smoke
cargo run -q --bin procon -- version
cargo run -q --bin procon -- hello

# 5. Frontend smoke
(cd packages/frontend && bun run dev)   # http://127.0.0.1:5173
```

## Validation entry point

`just` を推奨。なければ `justfile` を読んで直接 `cargo` / `bun` を呼ぶ。

```bash
just --list             # 利用可能な recipe
just fmt                # cargo fmt + bun format
just check              # fmt --check + clippy + lint + typecheck
just test               # cargo test + bun test
just test-integration   # integration layer
just ci                 # ローカル CI と同じフル検証
```

## 個別コマンド

### Rust

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p engine                    # crate 単位
cargo build --release --workspace
```

### Frontend

```bash
(cd packages/frontend)
bun run check          # tsc --noEmit
bun run lint           # ESLint
bun run format:check   # Prettier
bun test               # bun test runner
bun run build          # vite build (型チェックも走る)
bun run dev            # vite dev server (127.0.0.1:5173)
```

## ディレクトリ早見表

- `crates/domain` — domain types / invariants (no I/O)
- `crates/protocol` — wire format (JSON)
- `crates/engine` — match table / replay / server façade
- `crates/cli` — `procon` バイナリ
- `packages/frontend` — SolidJS + WebGPU
- `docs/` — architecture / development / release / security / recovery
- `docs/adr/` — ADR
- `.claude/skills/` — project-local Agent Skills

## よく使う git ワークフロー

```bash
# 新規 ticket branch
git fetch origin
git switch -c 123                # Issue 番号

# 初回 push + Draft PR
git push -u origin 123
gh pr create --draft --base release-0-2-0 \
  --title "[#123] <scope>: <terse summary>" \
  --body-file .github/PULL_REQUEST_TEMPLATE.md

# 状態同期
git fetch origin && git rebase origin/release-0-2-0

# Draft → Ready
gh pr ready
```

## デバッグ tips

- `RUST_LOG=engine=debug,domain=info` で tracing 詳細表示。
- WebGPU エラーは `console.error('WebGPU device lost', err)` 経由で出る。
- protocol mismatch が出たら `cargo run -q --bin procon -- hello | jq .` で実物 JSON を確認する。

## 次の作業を探す

- GitHub Projects の `Ready` column
- `gh issue list --label "status: ready"`
- ADR が提案中のものは `docs/adr/` の `proposed` 状態ファイル