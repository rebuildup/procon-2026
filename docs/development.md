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
cargo run -q --bin procon -- warmup        # pre-match solver timing

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

## Pre-match warmup (Issue #56)

`procon warmup` は試合直前にソルバ起動時間（cold / warm / 1 day smoke）を測定する ops コマンド。
`docs/contest/strategy.md` のリアルタイム要件（1日 ≒ 60秒）に対し、cold start が予算を食っていないかを毎回確認する。

3 つの pass:

| pass | 内容 | 用途 |
|---|---|---|
| `cold` | ソルバを毎回新規生成し `prepare() + decide()` を 1 回 | first-touch / page-fault 回帰の検知 |
| `warm` | 同一インスタンスを再利用、`decide()` のみ測定 | per-call アルゴリズム回帰の検知 |
| `smoke` | `day_steps` × `agents` 回 `decide()` を end-to-end | スケール時のみ顕在化する回帰の検知 |

運用レシピ:

```bash
just warmup                  # human-readable summary table
just warmup-json > bench.json # CI / アーカイブ用 JSON
cargo run -q --bin procon -- warmup --solvers greedy,astar --json
```

JSON レポートには p50 / p90 / max（µs 単位）とサンプル数が含まれる。スモーク p90 が `60_000_000 µs`（=60秒）を超えるソルバは試合投入前にチューニング必須。

## ソルバ評価（Issue #178）

解法の良し悪しを測るのは **offline の canonical evaluator** です。
`engine::MatchSim` を唯一の採点 oracle とし、公式スコア tuple
（`variety`, `daily_variety_sum`, `total_balls`, `response_time`）を
**辞書順**で比較します。詳細と標準手順は [`docs/evaluation.md`](evaluation.md) が SoT。

```bash
just eval-list          # 登録済み candidate（loop / multiloop / greedy / wait とパラメータ）
just eval               # 全 fixture × 既定 candidate、wait を baseline に
                        # → target/eval/{runs.jsonl, aggregate.json, report.md}

# 部分的に回す
PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" \
  cargo run -q -p bench --bin procon-eval -- \
    --cases small-8x8,series-many \
    --candidates 'loop:iterations=32:initial=greedy,greedy,wait' \
    --baseline wait --output-dir target/eval
```

対照: `procon tournament` は**公式サーバに対する wire/protocol smoke harness** で、
返るのは `protocol_progress = days_completed * 100 + sum(revisions)`（プロトコルが
どこまで進んだか）です。競技性能ではありません。

### offline candidate と live CLI の対応（Issue #170）

offline で選んだ `loop:initial=<value>` は、同じ意味のまま live 側に持ち出せます。
対応は 1:1 で、表の SoT は `crates/cli/src/initial_strategy.rs`
（docs の表はその写し）:

| offline candidate          | live `--loop-initial-strategy`（`tournament` / `coordinator` 共通） | `InitialStrategy`         |
|----------------------------|---------------------------------------|---------------------------|
| `initial=greedy`（既定）   | `greedy`（既定）                     | `GreedyNearestNeighbor`   |
| `initial=identity`         | `identity`                           | `Identity`                |
| `initial=reverse`          | `reverse`                            | `Reverse`                 |
| `initial=sweep-row`        | `sweep-row`                          | `SweepRow`                |
| `initial=sweep-col`        | `sweep-col`                          | `SweepCol`                |
| `initial=random:seed=N`    | `random:N`                           | `Random { seed: N }`      |

- seed だけ形が違います。offline は `seed=N` を独立パラメータにしますが、
  live は 1 フラグ = 1 戦略を保つため `random:N` に畳み込みます。
- 未知ラベルは**エラー**（以前の `Identity` への暗黙フォールバックは廃止）。
- どちらのサブコマンドも同じ parser を `value_parser` で共有しているので、
  protocol client は 1 本のままです。

```bash
cargo run -q --bin procon -- coordinator --loop-initial-strategy reverse
cargo run -q --bin procon -- tournament --solvers loop --loop-initial-strategy random:42
```

## 次の作業を探す

- GitHub Projects の `Ready` column
- `gh issue list --label "status: ready"`
- ADR が提案中のものは `docs/adr/` の `proposed` 状態ファイル