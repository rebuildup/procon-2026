# ADR-0011: 座標系は (col, row) タプル + EvenRowRightOffset の flat-top hex を採用する

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup
- Issue: #17 (Q1 連動)
- Depends on: ADR-0010

> 注: 当初 `0002-coord-system.md` を意図していたが、本 repo では `0002` が protocol-versioning、`0004` が sibling branch #11 (`hex-coord-and-non-torus`) で取得されるため、連番 `0006` を一時採用、後に ADR 番号重複解消のため `0011` にシフト。

## Context

`crates/domain` は 32×32 ヘックス盤面を扱う (strategy.md §5.1)。座標の **内部表現** を何にするかで、以下が連鎖的に決まる:

1. 隣接セル列挙 (`neighbors()`) の実装コスト
2. 距離計算の metric (axial vs cube vs offset)
3. JSON I/O 時の schema / payload サイズ
4. frontend (SolidJS + WebGPU) とのミラーコスト
5. 公式マップサンプル・Q&A 添付図との突き合わせやすさ

Q&A 第1弾 (Q1) は「座標系・向き」についての言及を含み、`docs/contest/strategy.md` から参照される (募集要項「エージェント」「移動と燃料」)。

主な選択肢:

| 案 | 概要 | 利点 | 欠点 |
|---|---|---|---|
| A | axial coord `(q, r)` | 数式が綺麗 / Red Blob Games の標準 / 3D cube coord への往復が線形 | 「行」と「列」が indirect、frontend 描画で row への写像が別途必要 |
| B | offset coord `(col, row)` + EvenRowRightOffset (本採用) | 「画面上の列・行」と一致 / frontend 直書き / 募集要項の図と 1:1 | 隣接セル計算が「偶奇で行で分岐」し分岐が 6 個 |
| C | cube coord `(x, y, z)` with `x + y + z = 0` | 距離・回転が対称 | 3 次元で冗長、JSON payload が太い、frontend 描画に逆変換 |

## Decision

**B を採用**: `HexCoord { col: u8, row: u8 }` + `MapLayout::EvenRowRightOffset` (flat-top, even-r-offset)。

### D-1. `HexCoord` の構造

```rust
pub struct HexCoord {
    pub col: u8,   // 0 .. 32
    pub row: u8,   // 0 .. 32
}

pub enum MapLayout {
    /// flat-top hex, 偶数行が右に +0.5 ずれる (Q1 回答)。
    EvenRowRightOffset,
}
```

- 内部表現は「画面と一致する直交 2 軸 (col, row)」 1 組のみ。
- `MapLayout` を分離することで、将来 odd-row offset や pointy-top への切り替え時の diff を局所化。
- 型で u8 を採用: マップ最大 32×32 (strategy.md §5.1)。`u8` で 0..32 の全範囲 + 境界判定の `>= WIDTH` チェックが自然。負値は型で排除。

### D-2. 偶数行右ずれの根拠 (Q1 連動)

- 公式フォーマット PDF / Q&A 第1弾 (Q1) で示される盤面図が「偶数行が右に 1/2 セル分シフト」して描かれている。
- これにより:
  - **隣接 6 マスのうち 2 マスが同 row** (`col ± 1`)、4 マスが上下の row (`col + {0, 1}`)。
  - 上下移動は「同じ col」または「col + 1」の 2 通り。`row % 2 == 0` で分岐。
- 募集要項の図 (盤面サンプル) と一致するため、サンプルマップを domain 側に取り込む際の座標写像が不要。

### D-3. 非トーラス (non-torus) の確定

- 募集要項「マップは有限」と読める。torus 化 (左右 / 上下ループ) は要件にない。
- `HexCoord::neighbors(&self, layout: &MapLayout) -> [HexCoord; 6]` は **盤外を `None` で返す** (`[Option<HexCoord>; 6]`) ことで、未到達セルを domain 側で表現。
- 境界チェックは `WIDTH / HEIGHT` 定数で 1 箇所に閉じ、`GameState::in_bounds(coord)` で再利用。

### D-4. 距離計算

- hex distance は内部で axial 表現 `(q, r) = (col - row/2, row)` に写像して計算し、結果を **`HexCoord` のまま返す** (`distance_to(&self, other: &HexCoord) -> u8`)。
- cube coord への変換は実装内部に閉じ、API に出さない。

### D-5. JSON / protocol との整合

- `HexCoord` は `{ col: u8, row: u8 }` で serde ラウンドトリップ。
- ADR-0002 (protocol-versioning) で確定した JSON v1 envelope と衝突しない (payload は `serde_json::Value` 経由)。

## Consequences

### Positive

- 募集要項・公式サンプル・Q&A 添付図との突き合わせが「画面 col,row」と完全一致。
- frontend (SolidJS + WebGPU) での描画: `(col, row)` を vertex buffer にそのまま流せる。WebGPU shader での offset 計算は `row % 2` の 1 命令で済む。
- 戦略アルゴリズム (A*、cluster) の隣接列挙が **table lookup** で済み、`match row % 2` の分岐は hot path でもない。
- `MapLayout` の enum 分離で、将来 pointy-top / odd-r への移行時に呼び出し側 (`Cell` / `GameState`) を変更せずに済む。

### Negative

- 隣接セル列挙が偶奇で分岐するため、初学者が読みにくい。
- axial / cube 系のドキュメント・資料より学習リソースが少ない (Q&A 連動のため公式が揃っていれば OK)。

### Mitigation

- `neighbors()` の偶奇分岐には **コメントで「Q&A Q1 に基づく」** と参照を残す。
- `MapLayout::EvenRowRightOffset` に doc-comment で「Q1」と「募集要項 §X」を引用し、ADR へのリンクも貼る。
- unit test で「(0,0), (5,3), (31,31)」の代表点 + 4 隅の境界 (`None`) を全網羅。

## Alternatives (検討したが採用しなかった案)

| 案 | 不採用理由 |
|---|---|
| Axial `(q, r)` 一本 | frontend 描画で `col = q + (r - r&1)/2` の写像が毎回必要。WebGPU ホットループに入る |
| Cube `(x, y, z)` | 3 次元。JSON payload が 1.5x。`x + y + z = 0` 不変条件を型で守れない (実行時チェック頼み) |
| Odd-row offset (even-r の逆) | Q&A Q1 が even-r を指すため公式サンプルと 1 ずれる |
| Torus 化 (左端 = 右端) | 募集要項が有限マップを示唆。torus 化すると「端のスポット」「端のゴール」挙動が変わり、戦略意味が変わる |

## Related ADRs / References

- ADR-0010: domain のゲームモデル 6 entity 統合定義 (本 ADR が D-1〜D-7 の前提)
- ADR-0002: protocol JSON v1 envelope との整合
- ADR-0001: `crates/domain` を I/O なし純粋 crate とする不変条件
- Sibling branch #11 の ADR-0004 (`hex-coord-and-non-torus`): 個別の hex 計算ヘルパ詳細
- `docs/contest/strategy.md` §5.1 (マップ最大 32×32 = 1024 セル)
- Q&A 第1弾 (Q1): `docs/pdf/第37回大会競技部門問い合わせと回答その１.pdf`
- 募集要項「エージェント」「移動と燃料」
- Issue: #17, #11