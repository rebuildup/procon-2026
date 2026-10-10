# 補給車をどの開始位置に置くか — 試合前の試算で選ぶ（Issue #428）

2026-10-10 夜、MacBook Air（Apple M5、10 コア、他セッションの計測で load 20〜70）。source は branch `428`。

## 結論

- `--kinds-auto` は補給車を必ずスロット 0 に置く。開始位置は盤面ごとに違うので、これは任意の 1 台を選んでいるのと同じ。
- 各スロットを補給車にした 1 試合を短い探索（`iters=8000`）で最後まで再生し、公式順（V → D → B）の平均で選ぶと、**本選型の 38 盤面で D +0.25/試合、B +0.9%**、他の盤面群 45 盤面でも悪化しない。
- 既定動作は変えていない。`PROCON_SUPPLY_SLOT_WINDOW_MS` を設定したときだけ動く。

## 道路の混み方の前提（`players = 1`）

オフライン評価は他チームの交通量を 0 と数える（`players = 10` なら自チームの 1/10）ので、道路が渋滞しない。Arena の公開 API（`GET https://procon37arena.online/api/matches/<id>`）から取った盤面で比べると、`players = 1`（全チームが自チームと同じ道路を使った場合の交通量）の再生が実戦の B を約 3% 以内で再現し、`players = 10` は 5〜25% 高く出る（Issue #428 の表）。試算も評価もすべて `players = 1` で行った。

## 評価盤面

| 群 | 数 | 中身 |
|---|---:|---|
| real16 | 2 | 本選 1 回戦第 3 試合、予行演習 |
| arena-official | 4 | Arena `terrain: official` の match 1997（16×16）、1998〜2000（24×24、燃料 60） |
| round1-like16 | 16 | branch `406` の `round1-like-406/boards` |
| tight-like-16 / 24 | 8 + 8 | `tools/gen_final_like.py … tight mirror`（1 回戦の枠を辺の長さで拡大、燃料 = 初日 step の 1.25 倍、seed 300〜） |
| arena（official 以外） | 29 | match 1900〜1997 のうち 7 日以下・初日 110 step 以下 |
| final-like-24 / 32 | 8 + 8 | 運営サンプルの枠（燃料 80 / 120）、seed 100〜 |

## 結果

**本選型 38 盤面**。試算は `fleet:iters=8000` の 1 試合を seed を変えて N 回（`results/trials-official-8k-12seeds.jsonl`）。採点は試算に使っていない `fleet:iters=60000:chains=4` の seed 42 と 7 の平均（`results/truth-official-seed*.jsonl`）。スロット 0 固定との比較：

| 試算の回数 | 勝 / 分 / 敗 | ΔD（38 盤面計） | ΔB |
|---:|---|---:|---:|
| 12 | 18 / 11 / 9 | +9.5 | +35.5（+0.9%） |
| 4（2 通り） | 14〜16 / 11〜12 / 11〜12 | +6.5 | +16.5〜+38 |
| 1（2 通り） | 16〜17 / 12〜15 / 7〜9 | +5〜+9 | +31〜+35 |

群別（12 回）：

| 群 | n | 別スロットを選んだ数 | 勝 / 分 / 敗 | ΔD | ΔB |
|---|---:|---:|---|---:|---:|
| real16 | 2 | 0 | 0 / 2 / 0 | 0 | 0 |
| arena-official | 4 | 3 | 2 / 2 / 0 | +1.0 | +0.6% |
| round1-like16 | 16 | 12 | 6 / 4 / 6 | +2.5 | +0.3% |
| tight-like-16 | 8 | 6 | 5 / 3 / 0 | +2.5 | +1.3% |
| tight-like-24 | 8 | 8 | 5 / 0 / 3 | +3.5 | +1.6% |

試算どうしの交差検証（6 seed で選び、別の 6 seed の `iters=8000` で採点、2 通り）は 19〜24 勝 7〜10 分 7〜10 敗、ΔD +14〜+16。

**他の盤面群 45 盤面**。実装そのもの（`slot_probe`、窓 3 秒）が選んだスロットを `fleet:iters=60000:chains=4:seed=42` で 1 試合ずつ（`results/pick-other-families.jsonl`）：

| 群 | n | 勝 / 分 / 敗 | ΔD | ΔB |
|---|---:|---|---:|---:|
| arena 16×16 | 18 | 11 / 4 / 3 | +2 | +1.4% |
| arena 24×24・32×32 | 11 | 5 / 3 / 3 | 0 | 0.0% |
| final-like-24 | 8 | 5 / 1 / 2 | +1 | +1.7% |
| final-like-32 | 8 | 5 / 1 / 2 | 0 | +1.1% |

本選 1 回戦の実盤面はスロット 0 が最良で、試算もスロット 0 を選ぶ（D 合計 2949 対 2795〜2912、79 試合）。Arena match 1997 はスロット 0 が最悪（初日の厳密な上限がスロット 0 で 16 玉、スロット 2 で 19 玉）。

## 窓に入る試算の回数

`slot_probe`、全スロットが同じ回数だけ終えた分（load 28 の状態）：

| 盤面 | 1.5 秒 | 5 秒 |
|---|---:|---:|
| 16×16・5 日 | 12〜25 | 43〜79 |
| 24×24・5 日 | 8 | 28 |
| 24×24・10 日 | 2 | 8〜12 |
| 32×32・5〜7 日 | 1〜2 | 4〜10 |

1 回でもスロット 0 固定より平均は良い。回数が増えるほど D の取り分が増える。

## 効かなかったもの

同じ調査で測って不採用にしたもの（Issue #428 のコメントに数値）：日末燃料の重み、翌日の貪欲推定の玉数、翌日推定の系列の重み（0 にすると D −11 / B +1%）、複数 seed の計画を残り日数の再生で選ぶ方式（`players = 1` の 17 盤面で 10 勝 7 敗と 5 勝 9 敗、差なし）、日境界の満タン反実仮想（B 0〜+2%）。

## 未検証

- 実戦（Arena / 公式サーバー）での確認。`POST /agent` の受付時間は公式サーバーで未確認（1 回戦では設定取得から試合開始まで約 60 秒あった）。窓を既定の 1.5 秒に足す形なので、設定する値は受付時間の内側に収めること。
- 32×32 の本選型盤面（生成はできるが、採点の計算時間の都合で未実施）。
- 補給 2 台以上の構成。

## 再現

```bash
cargo build --release -p bench --example round1_rollout --example slot_probe
T=docs/research/supply-slot-428/tools
python3 $T/gen_final_like.py /tmp/off 24 8 300 tight mirror
# 全スロットを 1 試合ずつ（採点用・試算用）
python3 $T/slots.py target/release/examples/round1_rollout truth.jsonl 5 'fleet:budget=official:seed=42:iters=60000:chains=4' /tmp/off/*.json
python3 $T/slots.py target/release/examples/round1_rollout trial.jsonl 3 'fleet:budget=official:seed=1:iters=8000' /tmp/off/*.json
python3 $T/slotcv.py 0.25 trial.jsonl -- truth.jsonl
# 実装が選ぶスロット
target/release/examples/slot_probe 3000 /tmp/off/*.json
```
