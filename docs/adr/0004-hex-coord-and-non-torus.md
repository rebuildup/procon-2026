# ADR-0004: Hex 座標系 (even-r オフセット) とマップはトーラスではない不変条件

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup
- Supersedes: -
- Related Issue: #11

## Context

全国高専プロコン競技部門「ヘキサうどん」(`docs/pdf/競技部門「ヘキサうどん_」のフォーマットについて.pdf`) のマップは六角セルを格子状に並べたものである。Issue #11 で `crates/domain` に座標系・境界・連結性に関するプリミティブを導入するにあたり、以下の点を確定させておく必要がある:

1. **座標系の表現**: `col, row` の offset 表現を使うか、axial/cube 表現を使うか。
2. **偶数行のオフセット方向** (Q1)。
3. **マップ端の扱い**: toroidal wrap (トーラス) なのか、有限平面なのか (Q2)。
4. **サイズ上限**: `8 ≤ W, H ≤ 32`。

## Q&A 原文の意図

- **Q1 (問い合わせその１)**: 偶数行の右ずれに固定。
  → "Even-r" offset layout のみが許可される。Odd-r は不可。
- **Q2**: マップはトーラスではなく、地形タイプ (平地/山地/道路) や境界がそのままの意味を持つ。
  → wrap-around してはならない。

## Decision

### D-1. 座標表現: `(col: u8, row: u8)` の offset 表現

Hex 座標の一般的な内部表現は axial `(q, r)` や cube `(x, y, z)` だが、

- protocol メッセージの可読性
- セル指定の単純さ
- 「col / row」というコンテスト公式表現との一致

を優先して **`(col, row)` offset 表現** を採用する。`u8` で十分 ([`MapSize::MAX`] = 32)。

axial/cube への内部変換は描画・経路探索時に行えばよい (Issue #11 scope 外)。

### D-2. Layout: `MapLayout::EvenRowRightOffset` のみ

Q1 により偶数行の右ずれに固定される。`MapLayout` は enum として表現するが、コンテストルール上は

```rust
enum MapLayout {
    EvenRowRightOffset,  // 唯一の許可値
}
```

の単一バリアントしか公開しない。これは将来 renderer 側でデバッグ用に odd-r 等を試す余地を

残しつつ、コンテストロジック側で取り違えるリスクを型で防ぐ設計。

### D-3. 境界は有限平面: `Bounds::check` / `Bounds::clamp`

Q2 によりマップはトーラスではない。よって

- 範囲外セルにアクセスしてはならない。
- `Bounds::clamp` は反対側に wrap せず、最大の有効セルに saturate する。
- ゲームロジックは `Bounds::check` で `Option<HexCoord>` を受け取り、`None` を「場外」として

  扱う。

`HexCoord` 自体は生値しか持たず、validate は `Bounds` 側に委ねる。これにより

「全座標が常に valid な前提」が崩れたときに offending 値を局所化できる。

### D-4. サイズ範囲: `MapSize::validate` で `8 ≤ W, H ≤ 32` を強制

Issue #11 AC 通り。`MapSize::new` は validate 失敗時に `CoreError::InvariantViolation` を返す。

## Consequences

### Positive

- コンテスト仕様 (Q1, Q2, サイズ範囲) が型と API に直接 encode され、取り違えがコンパイル時に防げる。
- `HexCoord` が `Copy + Hash + Eq + Serialize` を満たすので `HashSet<HexCoord>` 等を直接使える。
- 「場外」を `Option<HexCoord>` で扱うので path-finding 実装側が wrap を誤って実装しにくい。

### Negative

- `HexCoord` に layout / bounds の情報が乗っていないので、raw 値だけで `contains` 相当の判定を

  書こうとすると境界無視のバグが入りやすい。レビュー時に「`Bounds::check` を使っているか」を

  必ず確認する必要がある。
- offset → axial 変換を呼ぶたびに O(1) とはいえ計算が入る。描画 hot path では layout を一度

  取り出して使い回すのが望ましい。

### Mitigation

- ADR-0005 以降で「`HexCoord` を生で持たず `CellId(usize)` を持つ薄ラッパ型を後付けする」案を

  検討する余地がある (Issue #11 では保留)。
- `quality-gate` skill の domain review で「`Bounds::check` 経由か」を確認するチェックリストを

  追加する。

## Follow-ups

- Issue #11 の AC にある `Reachability` (連結性) は neighbor 計算と flood-fill を要するが、

  Issue #11 の scope は「型定義 + validate + 仕様トレース」のみで、`Bounds` で取り扱う境界と

  区別している。連結性 / 経路探索は別 Issue で `HexCoord::neighbors(layout)` を生やす。

- Issue #11 の Open Q: 標高表現 (平地/山地/道路) と地形占有 (spot / agent) は別 Issue。