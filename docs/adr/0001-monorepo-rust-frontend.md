# ADR-0001: Rust core と SolidJS frontend を同一 repository で管理する

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup

## Context

全国高専プロコン競技部門のプロジェクトでは、score / state を扱う高性能 Rust core と、可視化・対戦 UI を担う SolidJS + WebGPU frontend を開発する。

選択肢:

| 案 | 概要 | 利点 | 欠点 |
|---|---|---|---|
| A | 単一 repo (monorepo) | protocol 共有 / atomic commit / CI 一本化 / Issue 統合管理 | repo サイズが大きくなる |
| B | 別 repo + submodule | deploy 単位を分離できる | 同期コストが高い / cross-repo PR が必要 |
| C | Frontend を別 hosting (S3 等) | deploy 柔軟 | protocol drift のリスクが高い |

## Decision

**A を採用**。`crates/` と `packages/frontend/` を同一 repo で管理する。

理由:

- protocol 共有: Rust と TS の protocol 定義を同じ PR で更新できる。`PROTOCOL_VERSION` のズレは CI で必ず検出する。
- Issue / Project / release branch を統一できる。1 sprint で両方の進捗を管理しやすい。
- 受験期間 (数ヶ月) の短期決戦では、cross-repo coordination の overhead が大きい。

## Consequences

### Positive

- protocol 変更が atomic commit で入る。
- release 時に `release-x-y-z` branch 1 本で両者を versioning できる。
- CI が 1 workflow で完結。

### Negative

- repo サイズが大きくなる (Cargo registry + node_modules が両方含まれる)。
- Rust と Node の contributor が混在し、`.gitignore` がやや複雑。

### Mitigation

- `Cargo.lock` と `bun.lock` の両方を commit し、reproducibility を確保。
- `.gitignore` で `target/` / `node_modules/` / `dist/` を除外。
- CI で Rust job と Frontend job を分離し、fail-fast を切らない。

## Follow-ups

- 必要になったら ADR-0001 を見直し、`crates/` を別 repo へ切り出す条件を決める。
- repo サイズが肥大化したら sparse checkout を有効化。