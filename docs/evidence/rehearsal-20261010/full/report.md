# 予行演習マップ 2026-10-10 offline 比較

- 入力SHA256: 0878fa9f3a23d3942151a73cc3647cc7f96800fdb94134d161a76a3a58db107b
- 対象: fixture:rehearsal-20261010-105938
- source SHA: 54a0bb9a1af1252d6df83ec201574e18bf4023c0
- 公式順位 V > D > B > T (Tは小さい方)。対象外の bundled fixture は集計しない。
- 順位は反復中の最悪 V/D/B で付ける。T は V/D/B 同点のときだけ使う。
- この値は単独offline予測。全10チームの渋滞・車種提出30秒制限・HTTP受理は未確認。

| 順位 | 実験 | Kinds | Candidate | 回数 | 除外 | V/D/B 最悪 | V/D/B 最良 | 安定 | T 中央(ms) | 1日最大(ms) | 日別V |
|---:|---|---|---|---:|---:|---|---|---|---:|---:|---|
| 1 | det60k-1s-seed3 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=3 | 1 | 0 | 10/49/102 | 10/49/102 | yes | 2145.0 | 492.3 | 9-10-10-10-10 |
| 2 | det60k-1s | S,P,P,P | fleet:budget=official:iters=60000:chains=4 | 1 | 0 | 10/49/100 | 10/49/100 | yes | 1627.7 | 379.0 | 9-10-10-10-10 |
| 3 | ms1000-on-1s | S,P,P,P | fleet:budget=official:budget_ms=1000:adapt=on | 3 | 0 | 10/49/100 | 10/49/102 | NO | 3842.0 | 768.5 | 9-10-10-10-10 |
| 4 | live-1s | S,P,P,P | fleet:budget=official:budget_ms=10000 | 3 | 0 | 10/49/100 | 10/49/100 | yes | 3842.1 | 791.8 | 9-10-10-10-10 |
| 5 | ms2000-on-1s | S,P,P,P | fleet:budget=official:budget_ms=2000:adapt=on | 3 | 0 | 10/49/100 | 10/49/100 | yes | 3842.3 | 768.6 | 9-10-10-10-10 |
| 6 | ms1000-off-1s | S,P,P,P | fleet:budget=official:budget_ms=1000:adapt=off | 3 | 0 | 10/49/100 | 10/49/100 | yes | 5002.2 | 1000.6 | 9-10-10-10-10 |
| 7 | live-1s-seed3 | S,P,P,P | fleet:budget=official:budget_ms=10000:seed=3 | 2 | 0 | 10/49/98 | 10/49/100 | NO | 3842.4 | 768.6 | 9-10-10-10-10 |
| 8 | det60k-1s-seed2 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=2 | 1 | 0 | 10/49/97 | 10/49/97 | yes | 2056.9 | 473.6 | 9-10-10-10-10 |
| 9 | ms500-on-1s | S,P,P,P | fleet:budget=official:budget_ms=500:adapt=on | 3 | 0 | 10/49/97 | 10/49/99 | NO | 2502.0 | 500.5 | 9-10-10-10-10 |
| 10 | ms2000-off-1s | S,P,P,P | fleet:budget=official:budget_ms=2000:adapt=off | 3 | 0 | 10/49/97 | 10/49/102 | NO | 10002.2 | 2000.6 | 9-10-10-10-10 |
| 11 | ms250-on-1s | S,P,P,P | fleet:budget=official:budget_ms=250:adapt=on | 3 | 0 | 10/49/96 | 10/49/100 | NO | 1251.8 | 250.5 | 9-10-10-10-10 |
| 12 | ms250-off-1s | S,P,P,P | fleet:budget=official:budget_ms=250:adapt=off | 3 | 0 | 10/49/96 | 10/49/100 | NO | 1252.0 | 250.5 | 9-10-10-10-10 |
| 13 | live-s3 | P,P,P,S | fleet:budget=official:budget_ms=10000 | 3 | 0 | 10/49/95 | 10/49/97 | NO | 3841.9 | 768.8 | 9-10-10-10-10 |
| 14 | ms10000-off-1s | S,P,P,P | fleet:budget=official:budget_ms=10000:adapt=off | 3 | 0 | 10/49/95 | 10/49/104 | NO | 50002.4 | 10000.6 | 9-10-10-10-10 |
| 15 | det60k-1s-seed2026 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=2026 | 1 | 0 | 10/49/94 | 10/49/94 | yes | 1723.0 | 423.0 | 9-10-10-10-10 |
| 16 | live-1s-seed7 | S,P,P,P | fleet:budget=official:budget_ms=10000:seed=7 | 2 | 0 | 10/49/94 | 10/49/101 | NO | 3842.9 | 768.8 | 9-10-10-10-10 |
| 17 | ms5000-off-1s | S,P,P,P | fleet:budget=official:budget_ms=5000:adapt=off | 3 | 0 | 10/49/94 | 10/49/100 | NO | 25002.3 | 5000.6 | 9-10-10-10-10 |
| 18 | live-s2 | P,P,S,P | fleet:budget=official:budget_ms=10000 | 3 | 0 | 10/49/91 | 10/49/101 | NO | 3841.9 | 768.5 | 9-10-10-10-10 |
| 19 | live-s1 | P,S,P,P | fleet:budget=official:budget_ms=10000 | 3 | 0 | 10/49/91 | 10/49/101 | NO | 3842.1 | 769.3 | 9-10-10-10-10 |
| 20 | live-1s-seed1 | S,P,P,P | fleet:budget=official:budget_ms=10000:seed=1 | 2 | 0 | 10/49/87 | 10/49/100 | NO | 3851.5 | 776.6 | 9-10-10-10-10 |
| 21 | ms5000-on-1s | S,P,P,P | fleet:budget=official:budget_ms=5000:adapt=on | 3 | 0 | 10/49/86 | 10/49/100 | NO | 3842.0 | 768.6 | 9-10-10-10-10 |
| 22 | live-1s-seed2026 | S,P,P,P | fleet:budget=official:budget_ms=10000:seed=2026 | 2 | 0 | 10/48/105 | 10/49/96 | NO | 3842.7 | 768.7 | 8-10-10-10-10 9-10-10-10-10 |
| 23 | det60k-1s-seed7 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=7 | 1 | 0 | 10/48/103 | 10/48/103 | yes | 1779.8 | 406.4 | 8-10-10-10-10 |
| 24 | ms10000-on-1s | S,P,P,P | fleet:budget=official:budget_ms=10000:adapt=on | 3 | 0 | 10/48/103 | 10/49/100 | NO | 3842.1 | 768.5 | 8-10-10-10-10 9-10-10-10-10 |
| 25 | ms500-off-1s | S,P,P,P | fleet:budget=official:budget_ms=500:adapt=off | 3 | 0 | 10/48/101 | 10/49/94 | NO | 2502.0 | 500.5 | 8-10-10-10-10 9-10-10-10-10 |
| 26 | det60k-1s-seed1 | S,P,P,P | fleet:budget=official:iters=60000:chains=4:seed=1 | 1 | 0 | 10/48/99 | 10/48/99 | yes | 6667.3 | 1712.5 | 8-10-10-10-10 |
| 27 | trial-s1 | P,S,P,P | fleet:budget=official:iters=8000 | 1 | 0 | 10/48/98 | 10/48/98 | yes | 48.0 | 11.5 | 8-10-10-10-10 |
| 28 | det20k-1s | S,P,P,P | fleet:budget=official:iters=20000:chains=4 | 1 | 0 | 10/48/98 | 10/48/98 | yes | 551.8 | 137.3 | 8-10-10-10-10 |
| 29 | trial-1s | S,P,P,P | fleet:budget=official:iters=8000 | 1 | 0 | 10/48/96 | 10/48/96 | yes | 60.5 | 15.2 | 8-10-10-10-10 |
| 30 | live-1s-seed2 | S,P,P,P | fleet:budget=official:budget_ms=10000:seed=2 | 2 | 0 | 10/48/95 | 10/48/102 | NO | 3859.7 | 780.2 | 8-10-10-10-10 |
| 31 | trial-s2 | P,P,S,P | fleet:budget=official:iters=8000 | 1 | 0 | 10/48/91 | 10/48/91 | yes | 50.7 | 12.0 | 8-10-10-10-10 |
| 32 | trial-s3 | P,P,P,S | fleet:budget=official:iters=8000 | 1 | 0 | 10/48/88 | 10/48/88 | yes | 51.0 | 13.9 | 8-10-10-10-10 |
| 33 | live-psps | P,S,P,S | fleet:budget=official:budget_ms=10000 | 3 | 0 | 10/44/66 | 10/44/66 | yes | 2001.6 | 400.4 | 8-8-9-9-10 |
| 34 | det60k-psps | P,S,P,S | fleet:budget=official:iters=60000:chains=4 | 1 | 0 | 10/43/71 | 10/43/71 | yes | 1195.6 | 266.4 | 7-9-8-9-10 |
| 35 | trial-2s | S,S,P,P | fleet:budget=official:iters=8000 | 1 | 0 | 10/43/70 | 10/43/70 | yes | 43.6 | 9.8 | 7-9-9-9-9 |
| 36 | det20k-psps | P,S,P,S | fleet:budget=official:iters=20000:chains=4 | 1 | 0 | 10/43/69 | 10/43/69 | yes | 414.6 | 92.7 | 7-9-9-9-9 |
| 37 | trial-psps | P,S,P,S | fleet:budget=official:iters=8000 | 1 | 0 | 10/43/67 | 10/43/67 | yes | 51.2 | 13.3 | 7-9-9-9-9 |
| 38 | live-2s | S,S,P,P | fleet:budget=official:budget_ms=10000 | 3 | 0 | 10/43/66 | 10/44/68 | NO | 2001.5 | 400.4 | 7-9-8-9-10 7-9-9-9-10 |
| 39 | anytime-psps | P,S,P,S | anytime:budget_ms=5000:seed=42:budget=official | 2 | 0 | 10/42/57 | 10/42/57 | yes | 1702.5 | 391.1 | 7-8-9-9-9 |
| 40 | coverage-1s | S,P,P,P | coverage:budget=official | 1 | 0 | 10/33/46 | 10/33/46 | yes | 2.5 | 0.8 | 7-7-7-6-6 |
| 41 | anytime-4p | P,P,P,P | anytime:budget_ms=5000:seed=42:budget=official | 2 | 0 | 10/25/35 | 10/25/35 | yes | 372.9 | 217.1 | 10-6-3-3-3 |
| 42 | det60k-4p | P,P,P,P | fleet:budget=official:iters=60000:chains=4 | 1 | 0 | 9/43/59 | 9/43/59 | yes | 1024.9 | 238.2 | 7-9-9-9-9 |
| 43 | ms250-off-4p | P,P,P,P | fleet:budget=official:budget_ms=250:adapt=off | 2 | 0 | 9/43/58 | 9/43/58 | yes | 1251.3 | 250.4 | 7-9-9-9-9 |
| 44 | ms250-on-4p | P,P,P,P | fleet:budget=official:budget_ms=250:adapt=on | 2 | 0 | 9/43/58 | 9/43/58 | yes | 1251.4 | 250.4 | 7-9-9-9-9 |
| 45 | ms500-off-4p | P,P,P,P | fleet:budget=official:budget_ms=500:adapt=off | 2 | 0 | 9/43/58 | 9/43/58 | yes | 2501.5 | 500.4 | 7-9-9-9-9 |
| 46 | ms500-on-4p | P,P,P,P | fleet:budget=official:budget_ms=500:adapt=on | 2 | 0 | 9/43/58 | 9/43/58 | yes | 2501.5 | 500.4 | 7-9-9-9-9 |
| 47 | ms1000-off-4p | P,P,P,P | fleet:budget=official:budget_ms=1000:adapt=off | 2 | 0 | 9/43/58 | 9/43/58 | yes | 5001.4 | 1000.4 | 7-9-9-9-9 |
| 48 | ms1000-on-4p | P,P,P,P | fleet:budget=official:budget_ms=1000:adapt=on | 2 | 0 | 9/43/58 | 9/43/58 | yes | 5002.1 | 1001.4 | 7-9-9-9-9 |
| 49 | ms5000-on-4p | P,P,P,P | fleet:budget=official:budget_ms=5000:adapt=on | 2 | 0 | 9/43/58 | 9/43/58 | yes | 6826.3 | 1365.4 | 7-9-9-9-9 |
| 50 | live-4p | P,P,P,P | fleet:budget=official:budget_ms=10000 | 3 | 0 | 9/43/58 | 9/43/58 | yes | 6826.4 | 1365.8 | 7-9-9-9-9 |
| 51 | ms2000-on-4p | P,P,P,P | fleet:budget=official:budget_ms=2000:adapt=on | 2 | 0 | 9/43/58 | 9/43/58 | yes | 6826.5 | 1365.4 | 7-9-9-9-9 |
| 52 | ms10000-on-4p | P,P,P,P | fleet:budget=official:budget_ms=10000:adapt=on | 2 | 0 | 9/43/58 | 9/43/58 | yes | 6826.5 | 1365.4 | 7-9-9-9-9 |
| 53 | ms2000-off-4p | P,P,P,P | fleet:budget=official:budget_ms=2000:adapt=off | 2 | 0 | 9/43/58 | 9/43/58 | yes | 10001.4 | 2000.4 | 7-9-9-9-9 |
| 54 | ms5000-off-4p | P,P,P,P | fleet:budget=official:budget_ms=5000:adapt=off | 2 | 0 | 9/43/58 | 9/43/58 | yes | 25001.5 | 5000.4 | 7-9-9-9-9 |
| 55 | ms10000-off-4p | P,P,P,P | fleet:budget=official:budget_ms=10000:adapt=off | 2 | 0 | 9/43/58 | 9/43/58 | yes | 50002.6 | 10001.3 | 7-9-9-9-9 |
| 56 | trial-4p | P,P,P,P | fleet:budget=official:iters=8000 | 1 | 0 | 9/43/57 | 9/43/57 | yes | 29.3 | 6.9 | 7-9-9-9-9 |
| 57 | det20k-4p | P,P,P,P | fleet:budget=official:iters=20000:chains=4 | 1 | 0 | 9/43/55 | 9/43/55 | yes | 351.1 | 79.0 | 7-9-9-9-9 |
| 58 | anytime-1s | S,P,P,P | anytime:budget_ms=5000:seed=42:budget=official | 2 | 0 | 9/40/58 | 9/40/58 | yes | 1195.2 | 415.4 | 9-9-8-7-7 |
| 59 | coverage-psps | P,S,P,S | coverage:budget=official | 1 | 0 | 9/38/54 | 9/38/54 | yes | 4.0 | 0.8 | 7-7-8-9-7 |
| 60 | coverage-4p | P,P,P,P | coverage:budget=official | 1 | 0 | 9/22/37 | 9/22/37 | yes | 1.0 | 0.7 | 9-4-3-3-3 |

- kinds-auto 判定式（iters8000のoffline近似）: S,P,P,P
- kinds-auto 自体の1.5秒制限は別途 coordinator で検証。
- 採点対象: 118 / 118 run

