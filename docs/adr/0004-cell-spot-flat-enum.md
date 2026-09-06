# ADR-0004: `Cell` は flat enum、`Spot` は parallel struct

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup
- Issue: #12

## Context

Issue #12 で導入する地形型について、スポット付き平地を
`Cell::PlainWithSpot(Spot)` のような **追加バリアント** にするか、
`Cell::Plain` のまま `Spot` を **並列の型** として座標上に置くかの
2案を比較した。

## Options

| 案 | 概要 | 利点 | 欠点 |
|---|---|---|---|
| A | `Cell::PlainWithSpot(Spot)` を追加 | 地形とスポットが 1 型で表現できる | move/fuel コスト表の lookup ロジックが分岐。`Cell` のバリアント数が今後増える方向 |
| B | `Cell` を flat 4 バリアントに保ち、`Spot` を独立 struct にする | コスト表・lookup・match 文が単純。Q&A Q18 の "1セル1スポット" と直接対応 | 「この `Plain` セルにスポットがあるかどうか」を表現する上位データ構造が必要 |

## Decision

**B を採用**。

理由:

- 募集要項 表1 の move/fuel コスト表は **地形だけで確定** する。
  スポットの有無は別軸なので、enum の中にスポット payload を入れると
  `match cell { Cell::PlainWithSpot(s) => ..., Cell::Plain => ... }`
  のような分岐が全箇所に増える。
- Q&A Q18 ("1セル1スポット") は **同じセルにスポットが2つ以上乗らない**
  という意味であり、「スポットが `Cell` のバリアントで表現されるべき」
  という意味ではない。スポットは座標に対して 1:1 に紐づき、`Cell` とは
  独立に格納すれば十分。
- 1 ステップあたりの move/fuel はスポットの影響を受けないため、
  `Cell` のバリアントを増やすと **それらすべてが spot と無関係に分岐**
  するコストを払い続けることになる。

## Consequences

### Positive

- `Cell` は 4 バリアントに固定され、move/fuel の `match` および
  `MOVE_STEPS` / `FUEL_COSTS` の const 表が自明に整合する。
- スポットの追加・削除が `Cell` の表現に影響しない。
- 新しい地形 (将来的に水増し・荒地など) を追加するとき、
  `Cell` と `Spot` の **どちらか一方だけ** 変更すれば済む。

### Negative

- 「どの `Plain` にスポットがあるか」は別の `Vec<SpotAtCoord>` か
  `Map::spot_at(coord)` のような API で表現する必要がある (上位層
  `crates/engine` 側の責務)。

### Mitigation

- `Spot` に座標 payload を持たせる版 (`SpotAt { coord, series, max_stock }`)
  を engine 層で導入する想定だが、domain 層は **地形・スポットを
  直交する 2 型として扱う** ことで仕様トレースが崩れない。
- Q&A Q18 違反 (1 セルに複数スポット) は **domain テスト** で
  `Spot::new` の validation と組み合わせ、engine 層で重複配置を
  拒否する方針 (別途 ADR を切る可能性あり)。

## Follow-ups

- `crates/engine` で `Map` を導入するとき、座標 → `Cell` と座標 → `Spot` の
  2 系統を持つ `Map` 構造体を ADR-0005 で定義する。