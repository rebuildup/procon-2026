# ADR-0002: protocol は JSON v1 を採用し、major bump で breaking change を管理する

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup

## Context

Rust engine ↔ SolidJS frontend 間の通信プロトコルを設計する。

選択肢:

| 案 | 概要 | 利点 | 欠点 |
|---|---|---|---|
| A | JSON v1 + ProtocolVersion envelope | 人間可読 / TS 生成が不要 / schema 進化が緩やか | サイズとパースコストが msgpack / protobuf より大きい |
| B | MessagePack / Protobuf | サイズ小 / パース高速 | tooling 追加 / schema 変更が硬直 |
| C | 独自 binary | 最高速 | 開発コストが極めて高い |

## Decision

**A を採用**。

- `crates/protocol/src/lib.rs` で serde + JSON ベースの型を定義。
- `packages/frontend/src/lib/protocol.ts` で同じスキーマを TypeScript 側にミラー。
- `PROTOCOL_VERSION = { major: 1, minor: 0 }` を両者で一致させ、CI (`protocol-consistency` job) で守る。
- 各 envelope に `protocol` フィールドを含め、major 不一致時は `ProtocolError::UnsupportedVersion`。

## Consequences

### Positive

- frontend / backend の双方で人間可読 schema を維持できる。
- 受験期間中の短期開発で schema tooling の導入が不要。
- JSON なので `cargo run -q --bin procon -- hello | jq .` で動作確認が容易。

### Negative

- サイズ / パースコストは msgpack より大きい。WebGPU 描画のような hot path では payload を最小限に絞る必要あり。
- schema drift 防止に CI ガードが必須。

### Mitigation

- envelope 内の payload は `serde_json::Value` (TS: `unknown`) にし、schema 進化を緩やかにする。
- CI の `protocol-consistency` job で major version のズレを検出。

## Follow-ups

- performance critical な path が出てきたら、binary framing (Cap'n Proto 等) を再評価。
- schema-first に移行したくなったら JSON Schema + 生成コードを検討。