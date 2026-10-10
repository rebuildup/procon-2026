# 公式参考盤面 24×24・32×32 での現状ソルバー最大値の実測（Issue #423）

2026-10-10、MacBook Air（Apple M5 / arm64 / 10 コア、macOS 27.0）で実施。
盤面は公式の参考マップ（strategy-lab に埋め込みの `SAMPLES["24"]` / `SAMPLES["32"]`）を CaseSpec JSON 化した
[`official-ref-24.json`](official-ref-24.json)（SHA256 `ec0a47f0…cbac97`）と
[`official-ref-32.json`](official-ref-32.json)（SHA256 `1461ead5…2bd9487`）。
生記録は [`runs/`](runs/)（53 run、eval バッチ名ごとに 1 ファイル）。

Issue #423 への要約コメント: <https://github.com/rebuildup/procon-2026/issues/423#issuecomment-6097358432>

## 計測条件

- ソース: main `7a93ad76549397e21178476d472a6a070ecddfce`（v0.5.0）。`cargo build --release --locked -p bench --bin procon-eval`。
- `procon-eval --jobs 1`（直列）。候補 × 盤面 1 run ずつ。`--cases small-8x8` は評価器の仕様上必ず付くが、ここでは official-ref の記録のみを集計した。
- 車種は `--kinds 'S,P,P,P,P,P,P,P'`（パターンはスロット数で打ち切られるため、ref-24 では `S,P,P,P,P`、ref-32 では `S,P×7` になる）。
- `daySeconds` は参考マップに含まれないため **60 秒/日と仮定**（本選実測 #385 と同じ窓）。`players=10`・`busy=1`・`jammed=3` は参考マップの値のまま。
- offline 計測（`iters=` モード）は決定論的（同 config+seed で完全再現）。`budget_ms=` モードは wall-clock 依存で非決定的。
- 計測終盤は別セッションの計測と並走したため、表中の所要時間（elapsed）は目安。

## 結論

| 位置づけ | 盤面 | Candidate | V | D | B | 日次応答 |
|---|---|---|---:|---:|---:|---|
| **観測最大** | ref-24 | `fleet:budget=official:budget_ms=50000:adapt=off:seed=42` | **18** | **90** | **163** | 50.0s ×5 |
| offline 決定論の最良 | ref-24 | `fleet:budget=official:iters=200000:chains=8:seed=44` | 18 | 90 | 161 | ≤9.3s ×5 |
| **観測最大** | ref-32 | `fleet:budget=official:iters=500000:chains=8:seed=42` | **24** | **120** | **265** | ≤22.3s ×5 |
| live 再現型の最良 | ref-32 | `fleet:budget=official:budget_ms=50000:adapt=off:chains=1:seed=44` | 24 | 120 | 259 | 50.0s ×5 |
| 現行の本番設定（参考） | ref-24 / ref-32 | `fleet:budget=official:budget_ms=10000:adapt=on` | 18 / 24 | 90 / 120 | 155 / 257 | 3.1s / 10.0s ×5 |

- 順位は公式 4 段階タプル V（多様性）→ D（日次多様性和）→ B（玉数）の辞書順。
- 24 の B163 は wall-clock モードの**単一観測**（別 seed では 159）。±数 B の変動を含む。

## V/D は真の飽和（実測）

- ref-24 の系列数は **18**、ref-32 は **24** → V=18 / V=24 は**盤面の全系列を獲得**しており理論上限。
- D=90=5×18、D=120=5×24 も理論上限。5 日すべての日で全系列を毎日回収できている。
- したがってこの 2 盤面で残る改善軸は **B のみ**。

## 探索予算に対する B の応答（chains=1, adapt=off）

| 盤面 | 10s/日 | 50s/日 | 増分 |
|---|---:|---:|---|
| ref-24 | 155 | 163 | +8（+5.2%） |
| ref-32 | 257 | 259 | +2（+0.8%） |

- 50 秒設定は合法（60 秒窓に ~10s 余裕）だが、直列化・通信の揺らぎを考えると危険域。**budget_ms の大幅増は非推奨**。伸ばすなら 20000〜30000 が妥当域（未計測）。
- **上限は探索予算ではなく公式の 1 日 60 秒窓**: offline で iters=1M×chains=8 にすると日次応答が 60 秒を超え **invalid**（32 は日次 90〜206s、24 は日 3〜4 で 60〜76s）。完了させても B は下がる（32: 261 < 265）。

## chains=8 の効果と再現性

- **offline 決定論モードでは有効**: iters=200k で chains 1→8 は ref-24 B 152→159〜161、ref-32 B 254→261。
- **live（budget_ms）モードでは無効〜微減**: 50s/日で chains=1 と同値以下（24: 163→162/153、32: 259→258/255）。live は chains を並列コアで回すため 1 chain あたりの反復が減る（仮説）。
- seed 依存（offline・chains=8・200k・ref-32）: B 261/248/243/254（spread ±9）。「観測最大」は 4 seed 中の最良であり期待値ではない。
- #415（chains=8 の Arena 確認待ち）への示唆: offline 評価の利得は live wall-clock では出ない可能性。Arena 判定は live 側の検証としてそのまま意味がある。

## 車種

- 補給 **1 台先頭**が最良。`S,P…` と `P,S,…` は同点（ref-24: B159 vs 158、ref-32: B261 vs 261）。
- 補給 2 台は不利（ref-24: D88/B127、ref-32: B219）。

## 他候補

anytime / coverage / search / multiloop は D で 30 点以上離れ（ref-32: D 86〜120 vs fleet 120）、比較対象にならない。multiloop `assign=all` は大盤面で崩壊（V5）。`fleet` 一強。

## B が伸び悩む原因

**測定事実**:

- 在庫は各日の開始時に最大まで補充（公式ガイドライン確認済み）。1 日の理論収量は在庫総量 69 玉（ref-24）/ 141 玉（ref-32）。
- 観測 B/day は最大 ~33 玉（ref-24 の 47%）/ ~53 玉（ref-32 の 38%）。
- 探索予算を増やしても B の増分は +8 以下に飽和し、1M iters では窓超過で invalid（得なし）。
- 全 53 run で `fuel_short_count=0`・`illegal_action_count=0` — 燃料切れによる行動損失はこの 2 盤面では発生していない。

**仮説（要検証）**:

- B は探索品質ではなく**移動ステップ・燃料の物理制約**で頭打ち。1 巡回車の 1 日行動半径が盤面の 1/4 程度（#423 既記載）と整合する。未回収の在庫は遠隔側に偏っている可能性が高い。
- 改善の主戦場は構造変更（空間分担・専任割当、遠隔回収、日末位置評価 #428）。探索予算の追加投入はこの 2 盤面では完了済みと判断する。

## 推奨パラメータ（現時点）

- **live 本番**: `fleet:budget=official:budget_ms=10000:adapt=on:seed=42` を維持。
- **offline 最大値・候補比較の基準線**: ref-24 `fleet:iters=200000:chains=8:seed=44`、ref-32 `fleet:iters=500000:chains=8:seed=42`。
- これらの値は公式参考盤面 2 枚の**観測最大値**であり、理論上限（B は在庫の半分程度）でも本戦一般での最大値でもない。

## 再現方法

1. `docs/research/strategy-lab/hex-udon-strategy-lab.html` 内の `const SAMPLES=…;` をパースし、`W/H/cells/spots/starts/daySteps/fuelLimit/busy/jammed/players` を CaseSpec 形状（`map.cells` は `[H][W]`、`agents` は starts、`daySeconds` は 60×5）に整形する。本ディレクトリの 2 JSON がその産物。
2. `cargo run -q -p bench --bin procon-eval --release -- --cases small-8x8 --case-files <case.json> --candidates '<candidate id>' --kinds 'S,P,P,P,P,P,P,P' --output-dir <dir>`
3. `runs/` の各ファイルはバッチごとの `runs.jsonl` のコピー（バッチ名は #423 コメントの表と対応）。

## 関連

#423（親）・#317（Phase 1 総括）・#415（chains）・#428（日末位置評価）・`docs/evidence/rehearsal-20261010/`（16×16 の同类計測）
