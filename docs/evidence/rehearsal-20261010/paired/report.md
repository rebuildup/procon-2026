# 予行演習マップ 2026-10-10 offline 比較

- 入力SHA256: 0878fa9f3a23d3942151a73cc3647cc7f96800fdb94134d161a76a3a58db107b
- 対象: fixture:rehearsal-20261010-105938
- source SHA: 54a0bb9a1af1252d6df83ec201574e18bf4023c0
- 公式順位 V > D > B > T (Tは小さい方)。対象外の bundled fixture は集計しない。
- 順位は反復中の最悪 V/D/B で付ける。T は V/D/B 同点のときだけ使う。
- この値は単独offline予測。全10チームの渋滞・車種提出30秒制限・HTTP受理は未確認。

| 順位 | 実験 | Kinds | Candidate | 回数 | 除外 | V/D/B 最悪 | V/D/B 最良 | 安定 | T 中央(ms) | 1日最大(ms) | 日別V |
|---:|---|---|---|---:|---:|---|---|---|---:|---:|---|
| 1 | ab-ms5000-off | S,P,P,P | fleet:budget=official:budget_ms=5000:adapt=off | 8 | 0 | 10/49/98 | 10/49/100 | NO | 25006.9 | 5006.6 | 9-10-10-10-10 |
| 2 | ab-ms10000-off | S,P,P,P | fleet:budget=official:budget_ms=10000:adapt=off | 8 | 0 | 10/49/98 | 10/49/100 | NO | 50008.3 | 10013.3 | 9-10-10-10-10 |
| 3 | ab-ms1000-off | S,P,P,P | fleet:budget=official:budget_ms=1000:adapt=off | 20 | 0 | 10/48/102 | 10/49/102 | NO | 5003.6 | 1042.1 | 8-10-10-10-10 9-10-10-10-10 |
| 4 | ab-ms10000 | S,P,P,P | fleet:budget=official:budget_ms=10000 | 20 | 0 | 10/48/97 | 10/49/101 | NO | 3843.4 | 776.1 | 8-10-10-10-10 9-10-10-10-10 |

- kinds-auto 判定式（iters8000のoffline近似）: 未判定
- kinds-auto 自体の1.5秒制限は別途 coordinator で検証。
- 採点対象: 56 / 56 run

