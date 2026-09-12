# ADR-0007: AgentKind は Patrol / Supply の 2 種 + FuelPolicy は sealed trait 化

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup
- Issue: #17
- Depends on: ADR-0005, ADR-0006

> 注: 当初 `0003-agent-kind.md` を意図していたが、本 repo では `0003` が sprint-model で既に使用されている。連番 `0007` を採用する。

## Context

募集要項「エージェント」「移動と燃料」によると、本コンテストのエージェントは次の 2 種類:

| 種別 | 役割 | 燃料 |
|---|---|---|
| **巡回車 (Patrol)** | スポットからうどん玉を採取する | 1 日目の `daySteps` に基づき上限あり (1〜3 × daySteps の上限) |
| **補給車 (Supply)** | 巡回車に燃料を補給するタンクローリー | 燃料制限なし、燃料を消費せず移動可能 |

これを domain 層でどう型表現するか。候補は 3 つ:

| 案 | 形 | 利点 | 欠点 |
|---|---|---|---|
| A | `enum AgentKind { Patrol, Supply }` + 1 `Agent` struct | 共通フィールド (位置・チーム・ID) を 1 箇所に集約 / `Vec<Agent>` の均質性 | 種別固有の不変条件はメソッド内で分岐 |
| B | 種別毎 struct (`PatrolAgent` / `SupplyAgent`) + `enum Agent` | 種別固有のフィールドがそのまま struct に出る | 共通フィールド重複 / `Vec<Agent>` 内の match が必要 |
| C | 完全に別 entity (`Patrol` / `Supply` を `Agent` から独立) | 完全分離 | エージェント集合の統一処理不可 (e.g. 移動ステップ列挙) |

加えて、燃料モデル (`FuelPolicy`) をどう表現するか:

| 案 | 形 | 利点 | 欠点 |
|---|---|---|---|
| X | `enum FuelPolicy { Capped(u32), Unlimited }` | 単純 / payload が小さい | 「Unlimited の意味」を呼び出し側がコメントで守る必要 |
| Y | `pub trait FuelPolicy` (sealed) + struct 実装 | 振る舞い (残量更新・上限判定) を trait method に閉じ込め | struct 定義が増える |

## Decision

### D-1. `AgentKind` は 2 種 enum + 1 struct を採用 (案 A)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    Patrol,
    Supply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub kind: AgentKind,
    pub team: TeamId,
    pub pos: HexCoord,
    pub fuel_policy: Box<dyn FuelPolicy>, // sealed trait、後述
}
```

- `Vec<Agent>` で全エージェントを均質に保持できる (e.g. 「全 agent の次ステップを列挙」)。
- 種別固有の差は **メソッドで分岐** する。`match self.kind { Patrol => ..., Supply => ... }` で網羅性はコンパイル時に保証。
- 案 B (種別毎 struct) は **共通フィールド (id, team, pos) の 3 重定義** と、戦略実装側での「全 agent を 1 つの Vec で扱いたい」要求 (strategy.md §3.3 「複数ペアに分割」) に反するため不採用。

### D-2. `FuelPolicy` は sealed trait で表現 (案 Y)

```rust
mod sealed {
    pub trait Sealed {}
}

/// 燃料モデルの抽象。外部実装は禁止 (sealed)。
pub trait FuelPolicy: sealed::Sealed + std::fmt::Debug + Send + Sync {
    /// 現ステップの移動で消費される燃料を返す。
    fn consume(&self, movement: MovementKind, terrain: Terrain) -> u32;
    /// 補給 / 上限超過を反映した次状態を返す。
    fn after_step(&self, movement: MovementKind, terrain: Terrain) -> Box<dyn FuelPolicy>;
    /// 上限内かどうか。
    fn can_act(&self, movement: MovementKind, terrain: Terrain) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub struct PatrolFuel {
    pub remaining: u32,
    pub capacity: u32,
}
impl sealed::Sealed for PatrolFuel {}
impl FuelPolicy for PatrolFuel { /* ... */ }

#[derive(Debug, Clone, Copy)]
pub struct SupplyFuel;
impl sealed::Sealed for SupplyFuel {}
impl FuelPolicy for SupplyFuel { /* consume = 0 */ }
```

sealed 化する理由:

- 燃料モデルの **不正なサブクラス追加** (e.g. 1 日目で 3 倍化する実装) を compile-time で禁止。
- 案 X (`enum FuelPolicy { Capped(u32), Unlimited }`) は「Unlimited の意味」「上限超過時の挙動」を呼び出し側責任にし、戦略コードのいたるところに `match` を書かせる。sealed trait なら振る舞いも trait method に閉じ込められ、戦略側は `agent.fuel_policy.can_act(...)` 1 行で済む。
- 種別に紐づく不変条件 (Patrol は capacity を超える replenish 不能 / Supply は燃料を他エージェントに分け与える) を trait 内に閉じ込め、entity の肥大化を防ぐ。

### D-3. 燃料単位は `u32`、補給量は `Box<dyn FuelPolicy>` で扱う

- 募集要項の燃料上限は「`daySteps × 1〜3` 程度」 (`docs/contest/strategy.md` §3.1)。`daySteps` の最大値は 200 程度 (strategy.md §5.1) なので、上限は ~600。`u32` で余裕。
- `newtype Fuel(u32)` での過度な抽象化は **行わない**。理由: strategy.md §3.1〜3.3 の運用で「残量表示」「残量率」が多用されるため、表示側で `as f64 / capacity as f64` がそのまま書ける方が読みやすい。
- 補給の表現: Supply エージェントが Patrol に燃料を渡す行為は `Action::Refuel { from: AgentId }` で表現 (ADR-0005 D-4 参照)。

### D-4. 不変条件は `Agent::validate(ctx)` に集約

- Patrol: `remaining >= 0 && remaining <= capacity`、`can_act == false` のとき移動不可。
- Supply: `consume(_, _) == 0` を **不変条件として test で固定**。将来「Supply も平地 1/2 消費」のような仕様変更があれば、この ADR を **supersede** して別番号で記録する。

## Consequences

### Positive

- `Vec<Agent>` を統一的に扱えるため、戦略側の「全 agent の並列ステップ列挙」が単純。
- sealed trait により燃料モデルの外部拡張を禁止でき、安全性が compile-time で保証。
- 「Unlimited」「Capped」のような語彙的区別ではなく、振る舞いで trait method を呼ぶ形になり、戦略コードが `match` の連鎖から解放される。
- `Agent::validate` に種別不変条件を集約することで、engine 側の状態遷移が `validate → apply` の 2 ステップで純粋化。

### Negative

- `Box<dyn FuelPolicy>` は dyn dispatch のコストが乗る。Patrol のホットループ (`can_act` を毎ステップ呼ぶ) で **数 % のオーバーヘッド** の可能性。
- sealed trait は外部 crate からの拡張を禁じるため、プラグイン的な拡張 (e.g. テスト用「無限燃料 Patrol」) は本リポ内の `#[cfg(test)]` 経由でのみ可能。

### Mitigation

- ホットループのプロファイルを取り、`Box<dyn FuelPolicy>` が bottleneck になったら `AgentKind` で enum dispatch する案 (案 A の inline `match`) に **後退できる** よう、`FuelPolicy` の trait method 群を最小限 (`consume` / `can_act` のみ) に保つ。
- テスト時の代替実装は `#[cfg(test)] mod tests { ... }` 内に閉じ、本番ビルドに混ぜない (Rust の dead-code-elimination で消える)。

## Alternatives (検討したが採用しなかった案)

| 案 | 不採用理由 |
|---|---|
| 種別毎 struct + enum ラッパ (案 B) | 共通フィールド重複、`Vec<Agent>` が `Vec<EnumAgent>` になり呼び出し側が常に match。strategy.md §3.3 「複数ペアに分割」と相性が悪い |
| 完全分離 (案 C) | チーム内のエージェント集合を 1 つの `Vec<Agent>` で扱う要件 (ターン開始時の同時 step 列挙) が満たせない |
| `enum FuelPolicy` (案 X) | 「Unlimited の意味」「上限超過時の挙動」が呼び出し側責任になり、戦略コードが `match` で膨らむ。sealed trait で振る舞いを閉じ込めた方が保守性が圧倒的に高い |
| `newtype Fuel(u32)` 抽象 | 表示・残量率計算が頻発する戦略側で wrapper を剥がすコストが毎回乗る。`u32` 直渡しの方が読みやすい |

## Related ADRs / References

- ADR-0005: domain 6 entity 統合定義 (本 ADR が D-1〜D-7 の `Agent` の前提)
- ADR-0006: 座標系 (`HexCoord`, `MapLayout`) — `Agent::pos` の型
- ADR-0002: JSON v1 envelope での serde 互換性
- ADR-0001: `crates/domain` の純粋性不変条件
- `docs/contest/strategy.md` §3.1 (燃料経済の制約), §3.2 (巡回車の平地段数), §3.3 (結合パターン)
- 募集要項「エージェント」「移動と燃料」
- Issue: #17, #13