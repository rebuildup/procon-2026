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
