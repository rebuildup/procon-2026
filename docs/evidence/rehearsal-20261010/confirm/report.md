# 予行演習マップ 2026-10-10 offline 比較

- 入力SHA256: 0878fa9f3a23d3942151a73cc3647cc7f96800fdb94134d161a76a3a58db107b
- 対象: fixture:rehearsal-20261010-105938
- source SHA: 54a0bb9a1af1252d6df83ec201574e18bf4023c0
- 公式順位 V > D > B > T (Tは小さい方)。対象外の bundled fixture は集計しない。
- 順位は反復中の最悪 V/D/B で付ける。T は V/D/B 同点のときだけ使う。
- この値は単独offline予測。全10チームの渋滞・車種提出30秒制限・HTTP受理は未確認。

| 順位 | 実験 | Kinds | Candidate | 回数 | 除外 | V/D/B 最悪 | V/D/B 最良 | 安定 | T 中央(ms) | 1日最大(ms) | 日別V |
|---:|---|---|---|---:|---:|---|---|---|---:|---:|---|
| 1 | confirm-det60k-seed3 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=3 | 1 | 0 | 10/49/102 | 10/49/102 | yes | 1779.9 | 428.7 | 9-10-10-10-10 |
| 2 | confirm-det120k-seed2 | S,P,P,P | fleet:budget=official:iters=120000:chains=4:seed=2 | 1 | 0 | 10/49/102 | 10/49/102 | yes | 10385.9 | 3303.7 | 9-10-10-10-10 |
| 3 | confirm-det20k-seed11 | S,P,P,P | fleet:budget=official:iters=20000:chains=4:seed=11 | 1 | 0 | 10/49/100 | 10/49/100 | yes | 606.6 | 138.6 | 9-10-10-10-10 |
| 4 | confirm-det60k-seed42-again | S,P,P,P | fleet:budget=official:iters=60000:chains=4 | 3 | 0 | 10/49/100 | 10/49/100 | yes | 1707.8 | 405.6 | 9-10-10-10-10 |
| 5 | confirm-det60k-seed42 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=42 | 1 | 0 | 10/49/100 | 10/49/100 | yes | 2851.6 | 847.0 | 9-10-10-10-10 |
| 6 | confirm-det240k-seed2026 | S,P,P,P | fleet:budget=official:iters=240000:chains=4:seed=2026 | 1 | 0 | 10/49/100 | 10/49/100 | yes | 6878.5 | 1518.8 | 9-10-10-10-10 |
| 7 | confirm-det240k-seed3 | S,P,P,P | fleet:budget=official:iters=240000:chains=4:seed=3 | 1 | 0 | 10/49/100 | 10/49/100 | yes | 7004.9 | 1557.4 | 9-10-10-10-10 |
| 8 | confirm-det120k-seed7 | S,P,P,P | fleet:budget=official:iters=120000:chains=4:seed=7 | 1 | 0 | 10/49/98 | 10/49/98 | yes | 3505.8 | 843.2 | 9-10-10-10-10 |
| 9 | confirm-det120k-seed3 | S,P,P,P | fleet:budget=official:iters=120000:chains=4:seed=3 | 1 | 0 | 10/49/98 | 10/49/98 | yes | 3883.5 | 877.3 | 9-10-10-10-10 |
| 10 | confirm-det60k-seed11 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=11 | 1 | 0 | 10/49/97 | 10/49/97 | yes | 1708.1 | 397.9 | 9-10-10-10-10 |
| 11 | confirm-det60k-seed2 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=2 | 1 | 0 | 10/49/97 | 10/49/97 | yes | 1711.4 | 391.1 | 9-10-10-10-10 |
| 12 | confirm-det120k-seed12 | S,P,P,P | fleet:budget=official:iters=120000:chains=4:seed=12 | 1 | 0 | 10/49/96 | 10/49/96 | yes | 3364.6 | 768.5 | 9-10-10-10-10 |
| 13 | confirm-det240k-seed42 | S,P,P,P | fleet:budget=official:iters=240000:chains=4:seed=42 | 1 | 0 | 10/49/95 | 10/49/95 | yes | 9648.1 | 2206.6 | 9-10-10-10-10 |
| 14 | confirm-det60k-seed2026 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=2026 | 1 | 0 | 10/49/94 | 10/49/94 | yes | 1730.6 | 412.6 | 9-10-10-10-10 |
| 15 | confirm-det120k-seed2026 | S,P,P,P | fleet:budget=official:iters=120000:chains=4:seed=2026 | 1 | 0 | 10/49/94 | 10/49/94 | yes | 3986.2 | 1042.7 | 9-10-10-10-10 |
| 16 | confirm-ms1000-off | S,P,P,P | fleet:budget=official:budget_ms=1000:adapt=off | 10 | 0 | 10/49/94 | 10/49/102 | NO | 5001.7 | 1001.9 | 9-10-10-10-10 |
| 17 | confirm-det240k-seed12 | S,P,P,P | fleet:budget=official:iters=240000:chains=4:seed=12 | 1 | 0 | 10/49/94 | 10/49/94 | yes | 6871.5 | 1676.0 | 9-10-10-10-10 |
| 18 | confirm-det240k-seed2 | S,P,P,P | fleet:budget=official:iters=240000:chains=4:seed=2 | 1 | 0 | 10/49/94 | 10/49/94 | yes | 7678.3 | 2090.3 | 9-10-10-10-10 |
| 19 | confirm-ms2000-off | S,P,P,P | fleet:budget=official:budget_ms=2000:adapt=off | 10 | 0 | 10/49/93 | 10/49/102 | NO | 10002.0 | 2000.8 | 9-10-10-10-10 |
| 20 | confirm-det20k-seed2 | S,P,P,P | fleet:budget=official:iters=20000:chains=4:seed=2 | 1 | 0 | 10/48/103 | 10/48/103 | yes | 599.4 | 137.7 | 8-10-10-10-10 |
| 21 | confirm-ms250 | S,P,P,P | fleet:budget=official:budget_ms=250 | 10 | 0 | 10/48/103 | 10/49/100 | NO | 1252.4 | 258.2 | 8-10-10-10-10 9-10-10-10-10 |
| 22 | confirm-det60k-seed7 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=7 | 1 | 0 | 10/48/103 | 10/48/103 | yes | 1768.7 | 398.8 | 8-10-10-10-10 |
| 23 | confirm-ms500 | S,P,P,P | fleet:budget=official:budget_ms=500 | 10 | 0 | 10/48/103 | 10/49/100 | NO | 2502.0 | 502.3 | 8-10-10-10-10 9-10-10-10-10 |
| 24 | confirm-det120k-seed11 | S,P,P,P | fleet:budget=official:iters=120000:chains=4:seed=11 | 1 | 0 | 10/48/103 | 10/48/103 | yes | 3687.2 | 835.5 | 8-10-10-10-10 |
| 25 | confirm-det120k-seed42 | S,P,P,P | fleet:budget=official:iters=120000:chains=4:seed=42 | 1 | 0 | 10/48/103 | 10/48/103 | yes | 6221.8 | 2049.2 | 8-10-10-10-10 |
| 26 | confirm-det240k-seed11 | S,P,P,P | fleet:budget=official:iters=240000:chains=4:seed=11 | 1 | 0 | 10/48/103 | 10/48/103 | yes | 7381.4 | 1672.2 | 8-10-10-10-10 |
| 27 | confirm-det240k-seed1 | S,P,P,P | fleet:budget=official:iters=240000:chains=4:seed=1 | 1 | 0 | 10/48/103 | 10/48/103 | yes | 11779.3 | 3120.4 | 8-10-10-10-10 |
| 28 | confirm-det20k-seed2026 | S,P,P,P | fleet:budget=official:iters=20000:chains=4:seed=2026 | 1 | 0 | 10/48/102 | 10/48/102 | yes | 910.6 | 221.4 | 8-10-10-10-10 |
| 29 | confirm-det240k-seed7 | S,P,P,P | fleet:budget=official:iters=240000:chains=4:seed=7 | 1 | 0 | 10/48/102 | 10/48/102 | yes | 7395.1 | 1857.0 | 8-10-10-10-10 |
| 30 | confirm-det20k-seed7 | S,P,P,P | fleet:budget=official:iters=20000:chains=4:seed=7 | 1 | 0 | 10/48/101 | 10/48/101 | yes | 582.0 | 128.0 | 8-10-10-10-10 |
| 31 | confirm-det20k-seed1 | S,P,P,P | fleet:budget=official:iters=20000:chains=4:seed=1 | 1 | 0 | 10/48/101 | 10/48/101 | yes | 589.8 | 136.9 | 8-10-10-10-10 |
| 32 | confirm-ms10000 | S,P,P,P | fleet:budget=official:budget_ms=10000 | 10 | 0 | 10/48/101 | 10/49/100 | NO | 3842.6 | 771.4 | 8-10-10-10-10 9-10-10-10-10 |
| 33 | confirm-det60k-seed12 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=12 | 1 | 0 | 10/48/100 | 10/48/100 | yes | 1717.2 | 411.5 | 8-10-10-10-10 |
| 34 | confirm-det120k-seed1 | S,P,P,P | fleet:budget=official:iters=120000:chains=4:seed=1 | 1 | 0 | 10/48/100 | 10/48/100 | yes | 11516.4 | 2879.7 | 8-10-10-10-10 |
| 35 | confirm-det60k-seed1 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=1 | 1 | 0 | 10/48/99 | 10/48/99 | yes | 1656.7 | 386.0 | 8-10-10-10-10 |
| 36 | confirm-det20k-seed42 | S,P,P,P | fleet:budget=official:iters=20000:chains=4:seed=42 | 1 | 0 | 10/48/98 | 10/48/98 | yes | 568.2 | 140.7 | 8-10-10-10-10 |
| 37 | confirm-det20k-seed3 | S,P,P,P | fleet:budget=official:iters=20000:chains=4:seed=3 | 1 | 0 | 10/48/97 | 10/48/97 | yes | 566.2 | 133.9 | 8-10-10-10-10 |
| 38 | confirm-det20k-seed12 | S,P,P,P | fleet:budget=official:iters=20000:chains=4:seed=12 | 1 | 0 | 10/48/96 | 10/48/96 | yes | 662.8 | 169.7 | 8-10-10-10-10 |

- kinds-auto 判定式（iters8000のoffline近似）: 未判定
- kinds-auto 自体の1.5秒制限は別途 coordinator で検証。
- 採点対象: 85 / 85 run

