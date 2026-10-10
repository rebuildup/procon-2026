# 日末の燃料を翌日の必要量で評価する（`fuel_need` / `fuel_mix`、Issue #428）

2026-10-10 夜。評価はすべて `players = 1`（`docs/research/supply-slot-428/README.md` の前提）、`fleet:iters=60000:chains=4`、補給スロット 0。

## きっかけ

Arena match 2001（32×32・`terrain: official`・7 台・8 日・燃料 80）で convoy 59s が 10 位（23/177/234、1 位 23/184/321）。実戦ログでは 5 日目に巡回車 2 台が燃料 0 で日を開始した。

同じ枠（燃料 = 初日 step の 1.25 倍、巡回 6・補給 1）の盤面をオフラインで再生すると、`fleet` でも巡回車が燃料 2〜6 で日を終え、翌日 51〜96 tick 止まる。日境界で満タンにする反実仮想（`--oracle-refill`）の B は 9 盤面 2 seed 合計で 3870 → 4219（**+9.0%**）。24×24 以下の盤面ではこの差は 0〜2% で、燃料が効くのはこの枠に限られる。

補給 2 台（`S,S,P,…` / `S,P,P,S,…`）は B 1888 / 1873 対 1915 で改善しない。

## 変更

- `fuel_need=N`（既定 0 = 無効、従来と同一）：探索のスコアに「日末の燃料が翌日の消費見込みを満たす割合 × 巡回車 1 台の 1 日の玉数 × N%」を足す。消費見込みは 翌日の step 数 × 盤面の平均（燃料 / tick）を燃料上限で打ち切った値。補給車が同行している巡回車は満たしているとみなす。
- `fuel_mix=on`：chain が 2 本以上のとき、奇数番の chain はこの項なしで計画し、どの chain を出すかは必ず翌日の再計画で比べる（今日 + 翌日の系列・玉数）。燃料が効かない盤面で玉を落とさないための切り替え。
- `convoy` は同名の param を今日の内蔵 `fleet` に渡す。

## 結果（基準は param なしの同 seed）

| 盤面 | 設定 | n | 勝 / 分 / 敗 | ΔD | ΔB |
|---|---|---:|---|---:|---:|
| tight-like-32 ×8 + Arena 2001、seed 42・7 | `fuel_need=50` | 18 | 14 / 2 / 2 | 0 | +140（+3.6%） |
| 同 | `fuel_need=50:fuel_mix=on` | 18 | 15 / 2 / 1 | 0 | **+152（+3.9%）** |
| 同 | `fuel_need=100:fuel_mix=on` | 18 | 15 / 0 / 3 | 0 | +137 |
| tight-like-24 ×8、seed 42・7 | `fuel_need=50:fuel_mix=on` | 16 | 11 / 1 / 4 | **+7** | +45（+1.4%） |
| 実盤面 2 + Arena official 16/24 の 5 盤面、seed 42・7 | `fuel_need=50` | 14 | 4 / 3 / 7 | −1 | −14 |
| 同 | `fuel_need=50:fuel_mix=on` | 14 | 4 / 8 / 2 | +1 | +25 |
| 実盤面 2 + Arena 1997 + round1-like 16 + tight-like-16 ×8、seed 42 | `fuel_need=50:fuel_mix=on` | 27 | 10 / 9 / 8 | +5 | +22 |
| Arena（official 以外）29 + sample 枠 16、seed 42 | `fuel_need=50:fuel_mix=on` | 45 | 21 / 3 / 21 | −1 | +55（+0.6%） |

- `fuel_need` 単独は、燃料が効かない盤面（Arena 1999・2000）で 4 seed とも B を 1.5〜2% 落とす（`plain-real-and-arena-official-4seeds.jsonl`）。`fuel_mix=on` でこの損は消える。
- `fuel_need` の値：25 は +32 / +61、50 は +83 / +57、75 は +86 / +22（D −1）、100 は +53、200 は +12（seed 42 / 7、tight-32 の 9 盤面）。
- wall-clock の経路（`budget_ms=5000:adapt=off`）と `convoy` 経由でも違法・燃料不足 0 で完走（tight-like-32r-s301：211 → 217 / convoy 223）。

## 未検証

- 実戦。
- 同じ枠で反実仮想との差はまだ +5% 残る。補給車の合流の探し方（`serve` の近傍）は変えていない。
- `chains=8`（PR #415）との組み合わせ。
