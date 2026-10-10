# Track A — 探索予算カーブ（24×24 / 32×32、fleet / convoy）

親Issue: [#425](https://github.com/rebuildup/procon-2026/issues/425)（Round1 解析）。
実装は一切変更していない。**計測のみ**。

- provenance: `engine_replay`（`procon-eval`、公式 `engine::MatchSim`）
- ソース: `ebb0a1071ba14011f0fb6915348f851e8c37a49d`（= origin/403、release content、convoy 込み）
- バイナリ: `/Users/rebui/work/procon-verify-424/target/release/procon-eval`（事前ビルド済み）
- 盤面: campaign-311 fleet-371 の consumed-holdout 3 面（採用証拠には使わない予算調整専用）
  - `hold-24-a5-d9-short-s5014.json` — 24×24, 5 agent, 9 日, daySteps 57..75
  - `hold-32-a7-d10-mixed-s5017.json` — 32×32, 7 agent, 10 日, daySteps 101..253
  - `hold-32-a8-d10-short-s5020.json` — 32×32, 8 agent, 10 日, daySteps 66..102
- kinds: `S,P,P,P,P,P,P,P`（#417 証跡の `agent_kinds` と一致。5/7/8 agent に同一パターンで S 先頭+Patrol 埋め）
- 候補: `fleet:budget=official:budget_ms=<B>:adapt=off` と `convoy:budget=official:budget_ms=<B>:seed=42`、B ∈ {10000, 20000, 30000, 40000}
- 実行: `--jobs 1` 完全逐次（1 ソルバ走行のみ）。response time は軸4証拠としてそのまま引用可。
- 生データ: `/tmp/tracka-s50<board>-<cand>-<budget>/runs.jsonl`（`--output-dir` 24 分割）。artifact は本文書の表に要約済み。
- 注意: 一部 32×32 走行は load1 > 40 を回避するため待機を挟んだが、逐次性は維持（同時刻に走行した procon-eval は自走行 1 本のみ。他セッションの worker が一時的に在得した時間帯があるため、response の絶対値はノイズを含みうる。V/D/B は決定論側 (`adapt=off` fleet) かつ budget_ms 切り詰めのみに依存し、wall-clock ノイズの影響を受けにくい）。

## 1. 予算 × 盤面 × 候補（V / D / B、max daily response）

読み方: 公式辞書順 V → D → B → 応答。V/D が同じなら B 大が良い。応答は小さいほど良い（axis 4）。

### s5014（24×24, 5 agent, 9 日）

| budget_ms | fleet V/D/B | fleet maxday | convoy V/D/B | convoy maxday |
|---|---|---|---|---|
| 10000 | 20 / 180 / 291 | 10.0 s | 20 / 180 / 286 | 10.6 s |
| 20000 | 20 / 180 / 292 | 20.0 s | 20 / 180 / **293** | 20.3 s |
| 30000 | 20 / 180 / 290 | 30.0 s | 20 / 180 / 294 | 30.4 s |
| 40000 | 20 / 180 / 294 | 40.0 s | 20 / 180 / 290 | 40.6 s |

V=20, D=180 は全 8 走行で完全に不変（V=20 は 9 日× 全日飽和、D=180=20×9）。B は 286..294 の範囲で単調増加しない（budget を 4 倍しても ±4 玉の抖れ）。

### s5017（32×32, 7 agent, 10 日）— loss board

| budget_ms | fleet V/D/B | fleet maxday | convoy V/D/B | convoy maxday |
|---|---|---|---|---|
| 10000 | 27 / 270 / 1151 | 10.0 s | 27 / 270 / 1129 | 10.8 s |
| 20000 | 27 / 270 / 1155 | 20.0 s | 27 / 270 / 1138 | 21.7 s |
| 30000 | 27 / 270 / 1158 | 30.0 s | 27 / 270 / **1138** | 29.2 s |
| 40000 | 27 / 270 / **1155** | 40.0 s | 27 / 270 / 1143 | 39.1 s |

V=27, D=270 も全 8 走行で完全に不変。B は fleet 1151..1158（±0.6%）、convoy 1129..1143（±0.6%）。

### s5020（32×32, 8 agent, 10 日）

| budget_ms | fleet V/D/B | fleet maxday | convoy V/D/B | convoy maxday |
|---|---|---|---|---|
| 10000 | 27 / 270 / 676 | 10.0 s | 27 / 270 / 668 | 10.3 s |
| 20000 | 27 / 270 / 676 | 20.0 s | 27 / 270 / 668 | 20.6 s |
| 30000 | 27 / 270 / 680 | 30.0 s | 27 / 270 / 670 | 31.0 s |
| 40000 | 27 / 270 / 678 | 40.0 s | 27 / 270 / 667 | 41.0 s |

V=27, D=270 は全 8 走行で完全に不変。B は 667..680（±1%）。

全 24 走行を通じて `illegal_action_count=0 / rejected_response_count=0 / fuel_short_count=0`。

## 2. カーブがどこで平らになるか

- **24×24 (s5014)**: 10k で既に V/D 飽和（20/180）。B は budget に対して全く増えない（10k→40k で 291→294, +1%）。**10k で平ら**。
- **32×32 (s5017 / s5020)**: こちらも 10k で V/D 飽和（27/270）。B は 10k→40k で s5017 fleet 1151→1155（+0.3%）、s5020 fleet 676→678（+0.3%）。**10k で実質平ら**。32×32 は 24×24 より玉数の絶対値が大きいぶん、僅かな B 増はあるが、軸3の差は辞書順の第3位であり、V/D が同一ならタイブレークにしかならない。
- 日別 response 内訳から: convoy は 32×32 の 20k/30k/40k で日により budget cap に **張り付かない日** がある（例 s5017 convoy 20k: 11252/9713/5843 ms の日がある）。つまりこれらの日は cap に達する前に探索が自然収束している。cap に張り付く日（20.0-21.7 s）でも B はほぼ不変 → **余った探索はスコアを買えていない**。
- fleet (`adapt=off`) は全日・全 budget で cap に張り付き（s5017 10k でも全日 10.0 s）、かつ B は convoy と同程度。**cap を全部使っても B は増えない = 探索は遅くとも ~10k で収束**。

## 3. 判定: adapt 計算式の妥当性と convoy 本番 budget

- **24/32 いずれの盤面でも budget 10k 以上で V/D/B は実質一定**。探索の実効収束点は 24×24 も 32×32 も ~10k ms 付近にあり、32×32 に予算をScaling する必要はなかった（undercapped でも overcapped でもない: 現在の adapt 計算式は「小盤面は早めに打ち切る」目的を達成しており、32×32 で過小に打ち切って損をする盤面はこの 3 面では観測されなかった）。
- convoy 本番 `budget_ms=20000` は**維持を推奨**。30k/40k に上げても B は ±0.5% 未満で、response は線形に増える（= 軸4で実質的に不利、かつ load リスクを上げる）。20k は cap に張り付く日と張り付かない日が混在する「ちょうど余裕のある」点であり、24h 稼働の安定性を優先するなら 10k への**引き下げ**も B 損失 ~0.5% で検討の価値がある（s5017 convoy: 10k B=1129 vs 20k B=1138、-0.8%）。
- fleet の `adapt` 再調整は不要。`adapt=off` でも B は convoy と同水準（むしろ s5017/s5020 で convoy を B で上回る）ため、adapt の「小盤面早打ち切り」は response 軸の節約としてのみ効いている。

## 4. WSL 比較オペレータ用 eval_compare コマンド

branch-377 の `scripts/campaign/eval_compare.py`（`377` ブランチにのみ在得、`ebb0a10` worktree には無い。実行には `377` の checkout か、スクリプトだけ取り出して使う）。

```bash
# 377 ブランチ側で実行（procon-eval は ebb0a10 の release build を使う）
cargo build --release --locked -p bench   # 既に build済みなら不要
python3 scripts/campaign/eval_compare.py \
  --eval target/release/procon-eval \
  --name tracka-budget-curve \
  --case-files \
    docs/evidence/campaign-311/fleet-371/corpus-holdout/hold-24-a5-d9-short-s5014.json \
    docs/evidence/campaign-311/fleet-371/corpus-holdout/hold-32-a7-d10-mixed-s5017.json \
    docs/evidence/campaign-311/fleet-371/corpus-holdout/hold-32-a8-d10-short-s5020.json \
  --config 'fleet:budget=official:budget_ms=10000:adapt=off|kinds=S,P,P,P,P,P,P,P' \
  --config 'fleet:budget=official:budget_ms=20000:adapt=off|kinds=S,P,P,P,P,P,P,P' \
  --config 'fleet:budget=official:budget_ms=30000:adapt=off|kinds=S,P,P,P,P,P,P,P' \
  --config 'fleet:budget=official:budget_ms=40000:adapt=off|kinds=S,P,P,P,P,P,P,P' \
  --config 'convoy:budget=official:budget_ms=10000:seed=42|kinds=S,P,P,P,P,P,P,P' \
  --config 'convoy:budget=official:budget_ms=20000:seed=42|kinds=S,P,P,P,P,P,P,P' \
  --config 'convoy:budget=official:budget_ms=30000:seed=42|kinds=S,P,P,P,P,P,P,P' \
  --config 'convoy:budget=official:budget_ms=40000:seed=42|kinds=S,P,P,P,P,P,P,P' \
  --reference 'convoy:budget=official:budget_ms=20000:seed=42' \
  --jobs 1 \
  --out docs/evidence/campaign-311/eval-377/runs/tracka-budget-curve
```

- `--jobs 1` 推奨（response 軸を引用するなら必須。`budget_ms` は wall-clock で非決定論、`compare.json` に `deterministic: false` が付く）。
- FILLER_CASE (`fixture:openapi-sample`) は eval_compare が自動除去する。
- 8 config × 3 board = 24 run、本計測と同量。再実行ではなく `--no-run` で本計測の runs.jsonl を集約したい場合は、本計測の 24 分割 output-dir を `<out>/runs/<config-label>/runs.jsonl` の layout にマージしてから `--no-run` を使う（現状のままでは layout が違うため直接は不可）。
