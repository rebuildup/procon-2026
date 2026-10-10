# 追加測量進捗 (measurer: addenda-415)

作業 worktree: `.worktrees/eval-wsl-405` (branch eval-convoy-415)
binary sha256: `a6737883ae651d799bc7106fef0703c1e3a0b34ab1ad6ce8366f1c5eec14a6c7` (`target/release/procon-eval`)
開始: 2026-10-10 19:5x JST

## 測定対象 (チーム指示)

1. `b3-20s-seed42/` — convoy@20000 を 9 corpus 盤面へ拡張 (seed42, kinds S,P×7) + fleet@10s / chains=8 アンカー
2. `c2-20s-s2/` — convoy@20000 × 補給 2 台 (`S,S,P,P,P,P,P,P`) 9 盤面 + fleet@10s アンカー (seed42)
3. round1 match3 solo chains8 @20s (seed42/7) → `wsl-10s/round1-solo.jsonl` 追記
4. (余裕があれば) `b3-20s-s9012-s7/` — convoy@20000 seed7 s9012

先行情報: setE (hard convoy/fleet iters=120000, holdout fleet iters=60000) が並行稼働中。
重負荷 eval は setE 完了後に 1 件ずつ実行する。

## ログ

- [開始] setE 稼働中 (`hard-convoy-iters120k` 実行中) を確認。待機の間に軽量タスク 3 (round1_rollout chains8@20s) を開始。
- [19:44] **タスク 3 完了**: round1 match3 solo / observed / chains8 @20s を seed42・seed7 の 2 run 実施し
  `wsl-10s/round1-solo.jsonl` に追記 (48 → 60 行、各 run 5 day + summary)。label は候補 ID
  (`fleet:budget=official:budget_ms=20000:adapt=off:chains=8:seed={42,7}`)。

  | run | B | D | V | decide_ms | illegal/fuel_short/rejected |
  |---|---|---|---|---|---|
  | chains8@20s seed42 | 55 | 43 | 10 | 20012–20055 (全日 20s 使用) | 0/0/0 |
  | chains8@20s seed7 | 55 | 43 | 10 | 20012–20056 (全日 20s 使用) | 0/0/0 |

  比較 (同盤面 round1-solo.jsonl 既存): chains8@10s seed42/7 は B=55/D=43 → **20s にしても chains8 の対局盤面スコアは不変**。
  convoy@20s seed42/7 (B=54/D=46) に対し chains8@20s は B+1 / D-3 (辞書順で chains8 側が勝ち)。
  deadline 裕度: 無し — harness は remaining_ms=u32::MAX のため daily deadline 制約は eval 側で別途確認 (下記タスク 5)。
- [20:01] setE-F 完了確認 (`SET E-F COMPLETE` / `hard-convoy-iters120k`, `holdout-fleet-iters60k-1chain` 出力済み)。
- [20:09] **タスク 1 開始**: `wsl-10s/b3-20s-seed42/` — 候補 3 件
  (`convoy:budget=official:budget_ms=20000:seed=42` / `fleet:...budget_ms=10000:adapt=off:seed=42` / `...chains=8`) ×
  9 corpus 盤面 + 同梱 fixture 9 (b10s-seed42 と同一 18 case 軸) × kinds `S,P,P,P,P,P,P,P`, jobs 2, nice 15。
- [20:35] **タスク 1 完了**: 54 runs 全て `success` / scored、illegal=0, fuel_short=0, timeout=0。
  結果 `wsl-10s/b3-20s-seed42/{runs.jsonl,aggregate.json}`。

  corpus 9 盤面、convoy@20s の辞書順比較 (形式 V/dVs/B/rt):
  | case | convoy@20s | convoy@10s | fleet@10s | chains8@10s | vs10s | vsF | vsC8 | max daily_resp |
  |---|---|---|---|---|---|---|---|---|
  | s5001 | 13/91/189/111.0s | 13/91/188 | 13/91/188/70.0s | 13/91/188/70.0s | lose | lose | lose | 20.19s |
  | s5013 | 20/80/218/51.7s | 20/80/217 | 20/80/219/40.0s | 20/80/219/40.0s | lose | win | win | 20.36s |
  | s5016 | 20/200/473/173.8s | 20/200/472 | 20/200/471/100.0s | 20/200/475/100.0s | lose | lose | win | 20.60s |
  | s5020 | 27/270/671/168.6s | 27/270/666 | 27/270/673/100.0s | 27/270/677/100.2s | lose | win | win | 23.71s |
  | s5023 | 27/162/454/70.6s | 27/162/460 | 27/162/456/60.0s | 27/162/452/60.0s | win | win | lose | 16.96s |
  | s9003 | 13/107/114/73.5s | 13/105/115 | 13/106/113/90.4s | 13/106/113/90.2s | lose | lose | lose | 14.77s |
  | s9005 | 20/140/203/103.8s | 20/140/202 | 20/140/208/70.0s | 20/140/208/70.0s | lose | win | win | 18.36s |
  | s9008 | 20/180/261/143.7s | 20/180/278 | 20/180/268/90.0s | 20/180/255/90.0s | win | win | lose | 20.14s |
  | s9012 | 27/189/412/119.5s | 27/189/369 | 27/189/407/70.0s | 27/189/403/70.0s | lose | lose | lose | 20.74s |

  集計: corpus9 では convoy@20s vs convoy@10s = 2win/7lose、vs fleet@10s = 5win/4lose、vs chains8@10s = 4win/5lose。
  20s 予算は B を数個増やすが response_time (axis4) が 2 倍近く伸び、辞書順では軸1〜3 が同点の盤面で軸4 負けが dominant。
  同梱 fixture 9 盤面では convoy@20s vs fleet@10s = **9win/0draw/0lose**。
  **deadline 裕度**: convoy@20s の daily_response_us 最大は **23.71s (s5020)** — 60s 期限に対し余裕 ~36s、ただし budget_ms=20000 を
  上回る 23.7s が観測された (1 世代の post-processing を含む実測値。60s 期限では問題なし)。
  タスク 4 を先に実行 (軽量・1 盤面)。
- [20:38] **タスク 2 開始**: `wsl-10s/c2-20s-s2/` — 補給 2 台 kinds `S,S,P,P,P,P,P,P`、
  候補 `convoy:budget=official:budget_ms=20000:seed=42` + アンカー `fleet:...budget_ms=10000:adapt=off:seed=42` ×
  9 corpus + 同梱 fixture 9、jobs 2, nice 15。rider の 20s での安全確認 (illegal/fuel_short) を含む。
- [20:55] **タスク 2 完了**: 36 runs 全て `success`、**illegal=0 / fuel_short=0 / rejected=0 (両候補合計)** —
  補給 2 台 (`S,S,P,...`) × 20s は安全と判定。結果 `wsl-10s/c2-20s-s2/{runs.jsonl,aggregate.json}`。

  corpus 9 盤面 (convoy@20s vs アンカー fleet@10s、S2 kinds):
  | case | convoy@20s (V/dVs/B/rt) | fleet@10s | 辞書順 | max daily_resp |
  |---|---|---|---|---|
  | s5001 | 13/91/135/91.9s | 13/91/135/70.0s | lose | 17.02s |
  | s5013 | 20/80/179/50.1s | 20/80/179/40.0s | lose | 19.57s |
  | s5016 | 20/200/385/168.6s | 20/200/388/100.0s | win | 20.51s |
  | s5020 | 27/270/584/167.5s | 27/270/582/100.1s | lose | 21.31s |
  | s5023 | 27/162/397/69.4s | 27/162/399/60.0s | win | 16.90s |
  | s9003 | 13/63/63/51.6s | 13/63/63/90.0s | win | 8.86s |
  | s9005 | 20/140/178/106.2s | 20/140/181/70.0s | win | 20.20s |
  | s9008 | 20/180/231/150.4s | 20/180/229/90.0s | lose | 20.64s |
  | s9012 | 27/189/407/117.8s | 27/189/411/70.0s | win | 21.43s |

  集計: corpus9 = **5win/0draw/4lose**、同梱 fixture = 9win/0/0。
  補給 2 台は補給 1 台 (b3-20s-seed42, 同一候補 convoy@20s) に対し B が全面減
  (例: s5016 385 vs 473, s5020 584 vs 671, s9012 407 vs 412) — パトロール数減の直接コスト。
  **deadline 裕度**: daily_response_us 最大 **21.43s (s9012)**、60s 期限に対し十分。timeout 0。
- [20:57] **タスク 4 開始** (余裕分): `wsl-10s/b3-20s-s9012-s7/` — convoy@20000 seed7 s9012 1 盤面
  (+同梱 fixture 9, b2-20s-s9012 と同軸) × kinds `S,P,P,P,P,P,P,P`, jobs 2, nice 15。
- [21:05] **タスク 4 完了** → 指示タスク全件完了。s9012 convoy@20s:
  - seed7: V27/dVs189/**B397**/rt114.4s, max daily 20.52s, illegal=0, fuel_short=0, success
  - seed42 (b3-20s-seed42 参照): V27/dVs189/B412/rt119.5s → **seed7 が辞書順で勝ち** (軸1〜3 同点、B で 15 差)

## deadline 裕度まとめ (タスク 5 / 60s 期限との対比)

| run | 候補 | daily_response_us 最大 | 60s に対する余裕 |
|---|---|---|---|
| b3-20s-seed42 (18 case × 3 候補) | convoy@20s | **23.71s** (s5020) | ~36s |
| 〃 | fleet@10s / chains8@10s | (アンカー、10s 台前半) | 十分 |
| c2-20s-s2 (18 case × 2 候補) | convoy@20s (S2) | **21.43s** (s9012) | ~38s |
| b3-20s-s9012-s7 | convoy@20s seed7 | 20.52s | ~39s |
| round1-solo 追記分 | chains8@20s seed42/7 | 20.0–20.6s/日 (harness 側 decide_ms) | ~39s |

備考: budget_ms=20000 指定でも実測 daily_response_us は 23.7s まで観測された
(デシリアライズ・応答生成を含む実測)。60s 公式期限に対しては全条件で十分な裕度、timeout 0。
## 追加測定担当 2（P3 以降・引き継ぎ）

- [21:01] **P3 開始**: `wsl-10s/p3-persist-s9012/` — s9012 単盤面（+同梱 fixture 9）×
  `convoy:budget=official:budget_ms=10000:seed=42:forecast=persist`、kinds `S,P,P,P,P,P,P,P`、jobs 1、nice 15。
  判定基準: convoy@10s 既定（s9012 B369）との差。B+25 以上 = beam 完走時利得大 / +5〜25 = 監視 / +5 未満 = close。
- [21:03] **P3 完了**: 10 runs 全て success、illegal=0 / fuel_short=0 / rejected=0。ウォール ~90s。
  - s9012: V27/D189/**B407**、rt 37.9s、maxday 7.32s（60s 期限に対し十分）
  - 比較（同一 kinds S,P×7・同一 seed42・同一 budget 10s）: 既定 convoy@10s B369 → **+38**（判定基準 +25 以上 = beam 完走時利得大）
  - 参照: d2-persist の同一候補 s9012 は B387（+18）。単サンプル変動 ±20 を考慮すると実利得は +18〜38 の帯
  - 同一実行内の隣接値: fleet@10s B406 / chains8@10s B433 / convoy@20s B412 — P3(407) は半予算で convoy@20s にほぼ一致、fleet には並ぶが chains8 には届かず
  - 証跡: `wsl-10s/p3-persist-s9012/{runs.jsonl,aggregate.json}`
- [21:03] **(A) 開始**: `wsl-10s/b4-chains8-full/` — holdout24+hard12 × `fleet:budget=official:budget_ms=10000:adapt=off:chains=8:seed=42`、kinds `S,P,P,P,P,P,P,P`、jobs 3、nice 15。
- [21:21] **(A) 完了**: `wsl-10s/b4-chains8-full/` — holdout24+hard12+fixture9 = 45 runs、
  **全て success、illegal=0 / fuel_short=0 / rejected=0 / timeout=0**。ウォール ~18m（jobs 3、nice 15）。
  fleet chains8@10s（kinds S,P×7、seed42）の V/D/B（生成盤面 36）:
  | corpus | case | V/D/B | maxday |
  |---|---|---|---|
  | hard | s9003 | 13/106/113 | 10.01s |
  | hard | s9001 | 13/130/171 | 10.01s |
  | hard | s9002 | 13/117/208 | 10.01s |
  | hard | s9004 | 13/130/277 | 10.01s |
  | hard | s9007 | 20/139/172 | 10.01s |
  | hard | s9005 | 20/140/203 | 10.02s |
  | hard | s9006 | 20/140/230 | 10.08s |
  | hard | s9008 | 20/179/266 | 10.07s |
  | hard | s9011 | 27/189/335 | 10.10s |
  | hard | s9009 | 27/189/322 | 10.17s |
  | hard | s9010 | 27/189/264 | 10.12s |
  | hard | s9012 | 27/189/397 | 10.10s |
  | hold | s5007 | 13/91/132 | 10.16s |
  | hold | s5003 | 13/117/203 | 10.16s |
  | hold | s5005 | 13/130/340 | 10.10s |
  | hold | s5002 | 13/78/143 | 10.00s |
  | hold | s5001 | 13/91/188 | 10.00s |
  | hold | s5006 | 13/91/163 | 10.00s |
  | hold | s5004 | 13/91/200 | 10.02s |
  | hold | s5008 | 13/117/273 | 10.01s |
  | hold | s5015 | 20/200/353 | 10.01s |
  | hold | s5011 | 20/140/292 | 10.01s |
  | hold | s5009 | 20/80/279 | 10.01s |
  | hold | s5013 | 20/80/218 | 10.01s |
  | hold | s5010 | 20/120/210 | 10.00s |
  | hold | s5014 | 20/180/291 | 10.00s |
  | hold | s5016 | 20/200/473 | 10.01s |
  | hold | s5012 | 20/80/189 | 10.01s |
  | hold | s5019 | 27/108/321 | 10.01s |
  | hold | s5023 | 27/162/461 | 10.01s |
  | hold | s5017 | 27/270/1149 | 10.01s |
  | hold | s5022 | 27/270/925 | 10.01s |
  | hold | s5018 | 27/108/242 | 10.01s |
  | hold | s5021 | 27/189/727 | 10.01s |
  | hold | s5020 | 27/270/674 | 10.01s |
  | hold | s5024 | 27/189/537 | 10.01s |
  合計 B（生成 36 盤面）= **11941**（hard 2958 + hold 8983）。s9012 は B397（jobs 3 並列下）— b10s 単直列の chains8@10s B433 より低く、並列実行の B 変動を含む点に注意。
  **deadline 裕度**: maxday は全盤面 10.0〜10.2s（budget_ms=10000 の想定通り、jobs 3 の並列化込みでも 60s 期限に対し余裕 ~50s）。timeout 0。
  証跡: `wsl-10s/b4-chains8-full/{runs.jsonl,aggregate.json}`。
- [21:21] **指示タスク全件完了**（P3 / A 実施、B1〜B3 は先行担当が完了済み・再実行なし）。

## バッチ 1（パラメータ最適化・追加測定担当 2）

- [21:47] **E1 開始 (1/3)**: `wsl-corpus/param-chains8/` — holdout24+hard12+fixture9 ×
  `fleet:budget=official:iters=60000:chains=8:seed=42`、kinds `S,P,P,P,P,P,P,P`（chains4 baseline と同一構成）、jobs 6、nice 15。
  比較先: `wsl-corpus/{holdout,hard}-fleet-iters60k-chains4`（clockless baseline 済み）。
- [21:52] **E1 chains8 完了**: 45 runs 全 success、illegal=0 / fuel_short=0 / rejected=0 / timeout=0。ウォール ~6m（jobs 6）。
  chains4 baseline（iters=60000）との盤面別辞書順比較（openapi-sample 除外）: **19 win / 24 draw / 1 LOSE**。
  - **D-loss 1 件: hard s9008** 20/179/236 → 20/**175**/226（V 同点・D -4・B -10）。chains8 は辞書順リスクが実在（監視必要）
  - s9010 は逆に D 188→189 で B -15 の win。B 総量は全面増（例: s9012 373→383、s5016 464→472、s5017 1143→1144）
  - rt 最大 174.8s（s5017、10 日合計）/ maxday 最大 31.8s（s5017）— 60s 期限に対し余裕あり
  証跡: `wsl-corpus/param-chains8/{runs.jsonl,aggregate.json}`。
- [21:52] **E1 開始 (2/3)**: `wsl-corpus/param-chains12/` — 同一構成で `chains=12`。
- [22:02] **E1 chains12 完了**: 45 runs 全 success、illegal=0 等 0。ウォール ~9m（jobs 6）。
  - vs chains4 baseline: **23 win / 20 draw / 1 LOSE**（LOSE = hold s5010 B-only -2: 20/120/210→208）。**D-loss なし**（s9008 は 20/180/255 win に回復）
  - vs chains8: 10 win / 32 draw / 2 LOSE（いずれも B-only 微減 -3/-2）。B 合計 11813 → **11863**（+50）
  - B 総量は chains8 > chains4 を維持、chains12 はさらに微増。rt maxday 最大 35.2s（s5017）— 60s 期限内
  証跡: `wsl-corpus/param-chains12/{runs.jsonl,aggregate.json}`。
- [22:02] **E1 開始 (3/3)**: `wsl-corpus/param-chains16/` — 同一構成で `chains=16`。
- [22:11] **E1 chains16 完了 / E1 全体完了**: 45 runs 全 success、illegal=0 等 0。ウォール ~8m（jobs 6）。
  - **chains ラダー結論（gen36 B 合計・openapi 除外）**: chains4 = 11745 → chains8 = 11813 → chains12 = **11863** → chains16 = 11858。**ピークは chains12**
  - vs chains4 baseline: chains16 は 25 win / 15 draw / **4 LOSE**（すべて B-only、s9011 -12 / s5023 -19 を含む）
  - vs chains12: 12 win / 28 draw / 4 LOSE（B-only のみ、最大 -19）。**全ラダーを通じて D-loss は chains8 の s9008 のみ**（chains12/16 では出ず）
  - rt maxday 最大 ~19.4s 台（s5023）〜35s（chains12 s5017）— 60s 期限内
  - 判定メモ: chains12 が B 総量・辞書順リスクの両面で最良候補。D 非劣化の観点では chains8 の s9008 D-loss が残るため、本命は chains12・対抗 chains8
  証跡: `wsl-corpus/param-chains{8,12,16}/{runs.jsonl,aggregate.json}`。
- [22:11] **E2 開始 (1/3)**: `wsl-corpus/param-chains8-seed7/` — holdout24+fixture9 × `fleet:budget=official:iters=60000:chains=8:seed=7`、jobs 6、nice 15。
- [22:19] **E2 seed7 完了**: 33 runs 全 success、illegal=0 等 0。ウォール ~5m（jobs 6）。
  - vs chains8 seed42（holdout24）: 9 win / 15 draw / 8 LOSE（**すべて B-only**、最大 -19 = s5023）。holdout24 B 合計 8910 → 8898（**-12**）。D-loss なし
  - maxday 最大 19.0s（s5017）— seed7 は時間安全
- [22:30] **E2 seed123 完了 — 重大**: **9/24 盤面が invalid/timeout**（隠蔽なしで記録）:
  - invalid 6 件: official daySeconds 超過（day0〜1）。maxday s5022 **253.0s** / s5020 230.1s / s5023 188.2s / s5019 82.7s / s5012 72.7s — 60s の日窓を大きく超過
  - timeout 3 件: harness_deadline 到達で kill（s5017 605s / s5021・s5024 425s）
  - **seed123 + chains8 は探索時間が爆発**（seed42: maxday ≤31.8s、seed7: ≤19.0s に対し 188〜253s）。seed が実行時間の鲁棒性に支配的な影響を持つ
  - B 比較は無意味（失敗盤面 0 記録）、holdout24 有効 15 盤面の B 合計 6254 は参考値にもならず
  - 判定: **seed123 は本命から排除**。seed 選定には「実行時間安全性」も辞書順と同時に評価する必要あり
  証跡: `wsl-corpus/param-chains8-seed{7,123}/runs.jsonl`（seed123 の status/status_detail に実測値あり）。
- [22:39] **E2 証跡コミット**（b762ff6）。新セッションが引き継ぎ（binary 再ビルド済み sha256 `d5eb038d…`、source 2ed7906 = fuel_need/fuel_mix 取り込み後）。実行中計測なし・load 3.7→低下中を確認。
- [22:44] **F3 開始**: `wsl-10s/f3-fuel-corpus9/` — スクリーニング 9 盤面 × 6 候補（clocked @10s・seed42・kinds `S,P,P,P,P,P,P,P`・同一実行内アンカー）:
  - c4: `fleet:...budget_ms=10000:adapt=off:chains=4`（アンカー）
  - c4f: `+fuel_need=50` / c4m: `+fuel_need=50:fuel_mix=on`
  - c8: `+chains=8`（アンカー）/ c8f / c8m（同様）
  fuel_need=50 は fuel-need-428 README の本命値。判定: §6.2 の分解式 `gain(chains=c, mix=on) − gain(chains=c, mix=off)` を c=4/8 で取り、比 ≥2 盤面（s5016 2.72 / s5013 2.78 / s5020 3.0）で利得 ≈0 を確認する。chains8 組合せは PR #433 README の未検証項。jobs 2・nice 15。先に openapi smoke 6 候補 success（`wsl-10s/f3-smoke/`）。
- [22:5x] 並行: PR #435（bail=on / cluster=on、24×24/32×32 multi-rep 計測を WSL 担当へ委譲）の計測用 worktree `.worktrees/bail-435-verify`（eb2f5de）を作成し release ビルド開始。F3 完了後に bail 計測を実施。
- [22:53] **F3 中断（証跡未書き出し）**: 別担当（`.worktrees/pr428-verify`・official Arena case 計測）と重なり load 20 超。fleet clocked は worker あたりコア数スレッドを張るため並走すると双方污染。当方 run は nice 15 で押されていたため、開始 13 分で停止。**他担当完了後に F3 を単独再実行する**（ smoke は `wsl-10s/f3-smoke/` に済み・候補 ID 6 件は動作確認済み）。
- [23:07] **bail smoke 完了**: eb2f5de binary（sha256 `85119c9f…`）で `convoy:budget_ms=20000:seed=42` と `:bail=on` が openapi で V/D/B 同点（5/2/2・rt ~4.1s・illegal 0）。**cluster=on は eb2f5de に未実装**（PR #435 記載どおり実装中）— bail のみ計測可能。
- [23:20] **F3-offline 開始**: `wsl-corpus/f3-offline-fuel/` — スクリーニング 9 + 同梱 fixture 9 × 6 候補（**clockless `iters=60000`**・seed42・kinds `S,P×7`）。clocked 版は 3 セッション並走（`pr428-verify` / `measure-435` / `measure-63a7e5e`・load 19）で汚染されるため、負荷非依存の決定論モードを先行。chains8 × fuel_need 組合せ（PR #433 README 未検証項）と §6.2 分解式を clockless で先に確定する。jobs 2・nice 19。
- [00:1x] **F3-offline 完了（部分・19/108 セルが負荷起因 invalid/timeout）**: offline V/D/B は決定論なので success セルは有効、invalid は daySeconds 超過（nice 19 で他 3 セッションに押され wall 時間が日窓超過・`status_detail` に実測）。ウォール ~70 分。
  **部分確定（同一実行内アンカー・決定論・`wsl-corpus/f3-offline-fuel/runs.jsonl`）**:
  - **s9008 chains8 offline の D-loss が fuel_need=50 で回復**: c8 anchor `(20,175,226)` → c8 fuel `(20,180,263)`（**D+5・B+37**）/ c8 mix `(20,180,256)`。E1 の chains8 最悪ケース（s9008 dV−4 決定論再現）が消える。c4 mix は `(20,180,265)`（c4 anchor は invalid で同実行比較不可・E1 baseline 236 に対し +29）
  - **s9012（32×32・比1.0）c4: fuel B+56（373→429）/ mix B+44（417）**。D189 同点
  - 非拘束盤面は利得 ≈0（仮説 §6.1 どおり）: s5013（比2.78）fuel +1 / mix −1、s5016 c8 fuel 469 / mix 465（anchor invalid）
  - s5001: fuel −9 / mix −1〜3（B-only 微減・D 同点）。s9003: 全候補同点
  - 欠落セル（負荷 invalid）: s9005 c8×3、s9008 c4 anchor/fuel、s9012 c8 fuel/mix、s5016 c4×3+c8 anchor、s5023 c4 fuel/mix、s5020 全 6 — **静穏時に再実行**
  分析ツール: `results-405/tools/fseries_gain.py`（gain/mix_effect 分解・DV-LOSS 列挙）
- [00:2x] **F3-offline 確定集計（invalid セルにも決定論スコア記録あり・scored=False は wall 起因棄却のみ）**:
  盤面別 B 利得（fuel / mix、同 chain アンカー比・D 動きを併記）:
  | 盤面（比） | c4 fuel | c4 mix | c8 fuel | c8 mix |
  |---|---|---|---|---|
  | s9003 (1.0) | 0 | 0 | 0 | 0 |
  | s9005 | −4 | 0 | +3 | +1 |
  | s9008 | +10 (D+1) | +29 (D+1) | **+37 (D+5)** | +30 (D+5) |
  | s9012 (1.0) | **+56** | +44 | +24(inv) | +43(inv) |
  | s5001 | −9 | −3 | −7 | −1 |
  | s5013 (2.78) | +1 | −1 | +1 | 0 |
  | s5016 (2.72) | +4 | −1 | −3 | −7 |
  | s5023 (1.38) | −27(inv) | −11(inv) | −29 | −16 |
  | s5020 (3.0) | −2 | −13 | (timeout×2) | 〃 |
  - 合計（完了セル）: fuel-only +29(c4)/+26(c8)、mix +44(c4)/+50(c8)。**mix > fuel-only が両 chain で成立**（非拘束盤面の損を消す機構どおり）
  - **D 下落セル = 0**（完了セル全 34 対で dV ≥0。s5020 c8 timeout 2 セルは真欠落）
  - §6.2 分解式の判定: c4 と c8 で mix 利得が同幅 → **重複なし・独立寄与**。希釈なし
  - 監視セル: s5023 fuel-only −29（B-only・D 同点）、s5020 c8 未測定
  - 判定: `fuel_need=50:fuel_mix=on` は offline 辞書順で非劣化・B 合計 +44〜50。**chains8 offline の唯一の D-loss（s9008）を消す**点で chains8 採用の辞書順リスクを低減。検証 27 盤面と clocked 確認を経て初めて採用推薦
  欠落 2 セル（s5020 c8 fuel/mix）と inv セルの scored 再取得のため、静穏時に corpus9 再実行。
- [00:2x] **bail 計測開始**（PR #435 委譲分・worktree `.worktrees/bail-435-verify` eb2f5de・binary sha256 `85119c9f…`）:
  ラベル爆発盤面 4 枚（s5016 24×24 d10 / s5020 32×32 d10 / s5023 32×32 d6 / s9012 32×32 d7）×
  `convoy:budget_ms=20000:seed=42` vs `:bail=on`、jobs 1・nice 19。期待効果（#435）: B 同等のまま回答時間 20s→5s（axis4）＋爆発盤面で残予算を embedded fleet へ譲渡しての B 回収。証跡は bail-435-verify 側 `eval-bail-435/`、回収後に本 results へ取り込む。
- [00:3x] **bail 計測完了（8 corpus セル + fixture 18 セル・全 success・illegal/fuel_short/rejected 0・load 9-13 で実施）**:
  | 盤面 | parent B | bail B | 辞書順 | maxday |
  |---|---|---|---|---|
  | s9012 | 387 | 364 | loss（B −23・ノイズ境界） | 33.4s → **7.6s** |
  | s5016 | 452 | 445 | loss（B −7・ノイズ内） | 39.9s → **8.9s** |
  | s5023 | 413 | 452 | **win（B +39）** | 50.4s → **8.7s** |
  | s5020 | 615 | 672 | **win（B +57）** | 38.8s → **6.7s** |
  fixture 9 盤面は 8 draw + road-heavy-v2 B−1（byte 同等性の保証どおり小盤面は中性）。
  **axis-4 と期限安全性は構造的に確定**: parent は budget 20s を全日使い切り maxday 最大 50.4s（60s 日窓に接近・負荷下で超過リスク実在）に対し、bail は全盤面 maxday ≤8.9s。
  B は爆発盤面で #435 予測どおり +39/+57。s9012 −23 は負荷起因ノイズ域のため**静穏時 B 確認再実行を判定条件に追加**。正式分析ドキュメントはサブエージェントが `analysis/bail-435-wsl.md` に作成中。
