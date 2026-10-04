# Quality

project 固有の quality gate 設計。固定 bundle ではなく、stack / framework / runtime / release target に応じて compile する。

## Stack-aware profile

| Layer | 検証 | コマンド |
|---|---|---|
| Rust domain | fmt + clippy + unit | `cargo fmt --check`, `cargo clippy -p domain`, `cargo test -p domain` |
| Rust protocol | + serde round-trip + contract test | `cargo test -p protocol` |
| Rust engine | + tokio async integration + replay determinism | `cargo test -p engine` |
| Rust cli | + smoke (`hello` / `version`) | `cargo run -q --bin procon -- hello` |
| Frontend | tsc --noEmit + ESLint + Prettier + bun test + vite build | `bun run check`, `bun run lint`, `bun run test`, `bun run build` |
| Cross-crate | PROTOCOL_VERSION 一致 | `protocol-consistency` job |

## Verification taxonomy

変更 surface / risk に応じて required level を選ぶ。詳細分類は `.claude/skills/quality-gate/SKILL.md` を参照。

| 変更 | 必須 verification |
|---|---|
| `domain` の pure logic | unit |
| `protocol` の型追加 / 変更 | unit + contract (serde round-trip + frontend mirror) |
| `engine` の state machine | unit + integration (replay determinism) |
| CLI smoke | smoke (`version` / `hello` 出力確認) |
| Frontend WebGPU | unit (mock) + manual (ブラウザ) |
| Release 全体 | full applicable integration + release PR checklist |

coverage threshold は project-specific signal として扱うが、当面固定値は設定しない。

## Gates

### Worker gate (担当 scope で高速検証)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd packages/frontend && bun run check)
(cd packages/frontend && bun run lint)
(cd packages/frontend && bun run test)
```

### Ticket integration gate (clean candidate で full applicable verification)

```bash
just ci
# 中身: worker gate + integration test + smoke + cross-crate protocol check
```

### Release gate (release-x-y-z → main 前)

```bash
# Release PR description に貼るチェックリスト:
- [ ] cargo build --release --workspace
- [ ] cargo test --workspace --release
- [ ] bun run build (frontend dist を artifact upload)
- [ ] bun run test
- [ ] protocol-consistency
- [ ] (manual) WebGPU ブラウザ smoke
```

## False green 禁止

- `#[ignore]` / `.only` / `cargo test -- --skip` を release gate に持ち込まない。
- `|| true` で CI step を無条件成功させない。
- CI を `continue-on-error: true` で潰して green 偽装しない。

## 変更時の運用

- 新規 dependency 追加は ADR or Issue で `priority` / `reachability` を説明。
- `unsafe` を増やす変更は clippy.toml に `avoid-breaking-exported-api` を再評価。
- breaking change は必ず minor → major への version bump と release notes 反映。