# ADR-0009: エージェントの初期位置は「spot の無い平地」のみ

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup
- Issue: #13

## Context

`procon-2026` の試合開始時、各チームは 3〜8 体のエージェント（巡回車 /
補給車）をマップ上に配置する。募集要項および Q&A は「初期位置にスポッ
トがあってはならない」「初期位置は通行可能（= 平地）でなければならない」
と定めているが、具体的な型レベルでの表現は未決である。

選択肢:

| 案 | 概要 | 利点 | 欠点 |
|---|---|---|---|
| A | `Agent::new` に `cell: Cell` を渡し、不変条件で `Plain` かつ spot 無 しを強制する | 型で安全 | `Cell` / `Spot` 型に依存し Issue #12 をブロックする |
| B | `Agent::new` に `pos: Position` だけを渡し、初期位置ルールはエン ジン層の配置 API で検証する | #12 と独立して進められる | ドメイン層だけではルールを表現できない |
| C | `Agent` を `pos: Position` とし、`Agent::spawn_initial(...)` のような別コンストラクタを用意して「平地かつ spot 無 し」を別メソッドで表現する | 配置ロジックを後段に分離しつつドメインで意味を表現できる | API が二系統になる |

## Decision

**C を採用**。

- Issue #13 のスコープ（型定義 + 不変条件）に合わせるため、当面の
  `Agent` は `id` / `kind` / `pos` / `fuel` を直接保持する。`pos` は
  self-contained な `Position { col: u8, row: u8 }` を用いる。
- 初期配置の前提（spot の無い平地）は `Agent::new` の doc コメントお
  よびこの ADR で明記し、配置ロジック自体は `engine` 層（または専用
  Issue）に分離する。これにより Issue #11 / #12 と独立してドメイン
  型の API 面を確定できる。
- 将来 `Cell` / `Spot` が統合されたら、`Position` を `HexCoord` に、
  `MatchState` の `traffic` を `Cell` の派生物に差し替える（公開 API
  の `agent` モジュールは安定）。

## Consequences

### Positive

- Issue #13 を #11 / #12 にブロックされることなく進行できる。
- 初期配置ルールが「ドメイン層のドキュメント + エンジン層の配置
  API」という二段構えで表現され、責務分離が明確になる。
- 型レベルで `Supply` が無制限 / `Patrol` が `daySteps * 3` 上限と
  いう燃料ポリシーを表現できる（sealed `FuelPolicy`  trait）。

### Negative

- 現時点では `Agent::new` は `pos` の中身（cell 種 / spot 有無）を
  検証しない。配置ルールはエンジン層が必ず経由するという不変条件
  に依存する。
- Issue #11 / #12 の統合時に `Position` を `HexCoord` に置換する際
  に `agent` モジュール内を再エクスポートし直すマイナーな変更が
  発生する。

## Validation

- `cargo test -p domain`: 41 unit tests（fuel policy / agent count
  3-8 / 燃料上限 Q14 / serde round-trip / `MatchState` 不変条件）が
  全て緑。
- `cargo clippy --workspace -- -D warnings`: 警告 0。
- `cargo fmt --all`: 整形済み。
