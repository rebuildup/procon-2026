# ADR-0005: domain のゲームモデル 6 entity を統合的に定義する

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup
- Issue: #17
- Depends on: ADR-0001, ADR-0006, ADR-0007

> 注: 当初 `0001-domain-game-model.md` を意図していたが、本 repo では `0001-0003` が monorepo / protocol / sprint で既に使用されている。sibling branch (#11/#12) は `0004` を取得するため、本 ADR は連番 `0005-0007` を採用する。

## Context

`crates/domain` は本コンテストのルール・型・不変条件を保持する **純粋 domain crate** であり、外部 I/O・async・時刻を持たず、`engine` / `protocol` / `cli` に依存しない (ADR-0001 / `docs/architecture.md` 参照)。

現状 `crates/domain/src/state.rs` の `GameState` / `Turn` はプレースホルダに過ぎず、コンテスト固有の 6 entity は未定義である。本 ADR は、Issue #11 (#12 #13 で実装される) 種々 entity を **domain 層としてまとめてどう形作るか** を、後の逆転コストが高い判断に絞って確定する。

| Entity | 役割 |
|---|---|
| `Agent` | チームに属する 1 台の動く主体 (巡回車 / 補給車)。位置・燃料・所有チームを持つ |
| `Cell` | ヘックス盤面の 1 マス。地形・スポット所有・通行コストを持つ |
| `Spot` | うどん玉が湧く地点。系列・在庫・チーム独立 |
| `Series` | うどんの「種類」を表す同種スポット集合。Series 数 = 1 日の累積種類数の最大化対象 |
| `Traffic` | 道路セルの状態遷移 (順調 / 混雑 / 渋滞) と前日交通量履歴 |
| `Action` | 1 ステップでエージェントに出す命令 (移動 / 採取 / 待機 など) |

仕様の主な根拠:

- 募集要項「エージェント」「移動と燃料」(公式フォーマット PDF)
- Q&A 第1弾 (Q1: 座標系 / `docs/contest/strategy.md` 参照)
- `docs/contest/strategy.md` §2–4 (系列 / 燃料経済 / 道路渋滞)
- `docs/contest/strategy.md` §5.1 (マップ ≤ 32×32、スポット ≤ 32、エージェント 3〜8)

## Decision

### D-1. 6 entity を `crates/domain` に value object として置く

- すべて `Copy + Clone + Eq + Hash` (必要に応じて) の immutable value object として定義。
- `GameState` はこれらを集約する **read-only な射影** とし、可変状態は `engine` 側で `Arc<RwLock<_>>` に包む (architecture.md §"State model")。
- 1 entity は 1 ファイル (e.g. `crates/domain/src/agent.rs`) に閉じ込め、`pub use` のみを `lib.rs` に集約。

### D-2. `Cell` と `Spot` は「flat enum」にする (#12 と整合)

選択肢の比較:

| 案 | 形 | 利点 | 欠点 |
|---|---|---|---|
| A | 別 struct (`Cell` + `Option<Spot>`) | 概念が素直 | 「スポットでないセル」が `None` の乱発、パターンマッチが冗長 |
| B | 統合 enum `enum Cell { Plain(PlainCell), Spotted(SpottedCell) }` | スポット有無が型で表現され exhaustive match 強制 | 1 段の unwrap コスト |

**B を採用**。理由:

- 募集要項上「スポットは地形を持つ」+「地形マスにも玉は湧かない」が仕様上セットであり、モデルが「地形 + スポット所有」を常に組で扱うため enum の帰結が自然。
- `match cell { Plain(_) => ..., Spotted(_) => ... }` で網羅性がコンパイル時に保証され、戦略実装側の未処理ケースが残らない (strategy.md §5.2 のリアルタイム応答性に効く)。
- サイズ差は ~1 byte。32×32 = 1024 cell で 1 KiB、無視できる。

### D-3. `Agent` は種別差を `enum AgentKind` で表現 (詳細は ADR-0007)

- 同一 `Agent` struct に `kind: AgentKind` を持ち、種別固有の不変条件は `Agent::validate(&self, ctx)` に閉じ込める。
- 詳細・根拠は ADR-0007 (`0007-agent-kind.md`) を参照。

### D-4. `Action` は closed enum + payload struct

- 1 つの `enum Action { Move(Direction), Take(SpotId), Refuel { from: AgentId }, Wait }` を domain に置く。
- payload は小さく、JSON 化されても数百 byte 以下。
- 「不正な Action」は domain で `CoreError::IllegalMove` に変換 (状態機械は engine 側)。

### D-5. `Series` は別 entity として独立

- 「`Spot` の同値類」として lazy に計算すると、所有コストが経路探索ごとに再計算される。
- 入力マップ確定時に `Vec<Series>` を構築し、`Spot -> SeriesId` の lookup table を持つ。
- 系列 ID の安定性は「同じ入力マップに対して同じ ID 系列」を保証 (テスト容易性・replay 整合性)。

### D-6. `Traffic` は道路セルに紐づく状態機械

- `Traffic { level: Smooth | Crowded { traffic_yesterday } | Jammed { traffic_yesterday, traffic_day_before } }`。
- 状態遷移は前日・前々日の **全チーム合算** 交通量で決まり (strategy.md §4)、domain では純粋関数 `Traffic::next(prev, traffic_volume) -> Self` として表現。
- 閾値 `crowded_threshold` / `jammed_threshold` は `MapLayout` 経由で与えられる定数 (strategy.md §4 「閾値 1〜5 / 2〜10」)。

### D-7. 不変条件違反の表現

- `CoreError::InvariantViolation(&'static str)` を `lib.rs` で既に定義済み。
- 5 entity いずれの不変条件違反も、この error を返すことで domain 層の統一エラー型に集約する。
- 「ペイロードに構造化データを入れたい」要求は、engine 側で `InvariantViolation { source: &'static str, context: Value }` を **wrapper** して吸収する (domain 層に JSON 依存を持ち込まない)。

## Consequences

### Positive

- 6 entity の責務境界が ADR レベルで明文化され、#11 / #12 / #13 の実装が逆方向に進みにくくなる。
- `Cell` を flat enum にしたことで「スポット未処理セル」がコンパイル時に検出される。
- `Traffic` を domain の純粋関数にしたことで、Monte Carlo 系のテストが state なしで書ける。

### Negative

- 1 entity を 1 ファイルにする方針は、現状の `state.rs` 集約と比べてファイル数が増える。
- `Cell` の enum 化は flat struct からの書き換えコストを #12 に押し付ける。

### Mitigation

- `lib.rs` の `pub use` ハブで re-export をまとめ、外部から見える名前空間をフラットに保つ。
- enum 化に伴う migration は #12 の acceptance criteria に明示し、PR で「`Option<Spot>` を消す」差分を 1 commit に収める。

## Alternatives (検討したが採用しなかった案)

| 案 | 不採用理由 |
|---|---|
| `Cell` / `Spot` 別 struct + `Option<Spot>` | スポット未保有マスが常に存在し、`match` 網羅性保証が弱い。実装中に `if let Some(...)` の重複が起きやすい |
| `Series` を `Spot` のメソッドで lazy 計算 | 戦略アルゴリズムの hot path で毎回 `O(spot)` のコストが乗り、リアルタイム要件 (strategy.md §5.2) に抵触 |
| `Traffic` を `Cell` 内に格納せず外部 map | 「道路セルか否か」の判定が呼び出し側責任になり、domain の網羅性が崩れる |
| `AgentKind` を種別毎 struct に分割 (ADR-0007 で詳細比較) | 共通フィールドの重複・`Vec<Agent>` の均質性低下・match の網羅性低下 |

## Related ADRs / References

- ADR-0001: Rust core + SolidJS frontend の monorepo 構成
- ADR-0002: protocol は JSON v1 + ProtocolVersion envelope
- ADR-0003: sprint = 1 target semantic version
- ADR-0006: 座標系 (HexCoord, MapLayout: EvenRowRightOffset)
- ADR-0007: AgentKind (Patrol/Supply) + FuelPolicy (sealed trait)
- Sibling branches が作成する ADR-0004 (`hex-coord-and-non-torus` / `cell-spot-flat-enum`): 個別 entity のローカルな詳細決定
- `docs/contest/strategy.md` §2–5 (系列 / 燃料 / 道路 / リアルタイム要件)
- `docs/contest/team-context.md` (並走トラック方針)
- Issue: #17 (本 ADR), #11, #12, #13