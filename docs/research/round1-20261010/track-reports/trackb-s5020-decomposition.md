# Track B (#426): s5020 32×32 convoy 失点分解（実測）

- 日時: 2026-10-10 20:15–21:35 (JST)、worktree `procon-verify-424` @ origin/403 (ebb0a10)
- 盤面: `hold-32-a8-d10-short-s5020.json`（32×32、8台、10日、28スポット/27系列、daySteps 66..102、燃料 246、daySeconds 60×10）
- 車種: `S,P,P,P,P,P,P,P`（供給1・巡回7。1S ゲート上この盤面の実戦は fleet）
- 計測: `procon-eval --jobs 1` シーケンシャル + instrument 例 `crates/bench/examples/trackb_diag.rs`（solver 本体は無変更、`Cargo.toml` に `tracing-core` を追加したのみ）
- 負荷: load1 7〜45 の変動。budget_ms 系候補は壁時計で打ち切るため数値に ±2〜12 B の揺れがある

## 1. 総合結果（この環境での再測）

| 候補 | V | D | B | 応答/日 |
|---|---|---|---|---|
| `fleet:budget=official:budget_ms=10000:adapt=off` | 27 | 270 | **679**（diag 674） | 10.0s ×10 |
| `convoy:…:budget_ms=20000:seed=42`（handoff 設定） | 27 | 270 | **672**（2回とも） | 20.0s ×7 + 減衰 |
| `convoy:…:budget_ms=20000:labels=4000000` | 27 | 270 | 669 | 20.1s ×9 |
| `convoy:…:budget_ms=20000:width=48:depth=12` | 27 | 270 | 665 | 20〜22s |
| `convoy:…:budget_ms=30000:seed=42` | 27 | 270 | 670 | 30s ×7 |
| `convoy:…:budget_ms=59000:labels=4000000` | 27 | 270 | 671 | 59〜60s ×8 |
| `convoy:budget=official:seed=42`（時計なし） | — | — | **timeout**（605s で kill） | >60s/日 |

#426 記載の −37（545 vs 582）は本環境では再現されず、convoy 20s でも −2〜−12 B 程度。
ただし結果の性質は同じ: **convoy はこの盤面で内部探索が一切機能しておらず、実質 fleet 単体**（後述）。
差は環境負荷による fleet 側の壁時計揺れ（#426 の測定では convoy 10s vs fleet 10s、本測定では convoy 20s vs fleet 10s）。

## 2. Instrument 結果: pool は全日 1（最大の発見）

`convoy day plan` tracing（`convoy_solver.rs:1157`）を 20s・10s・labels=4M の 3 設定で全日収集:

```
20s  設定: day=0..9 すべて pool=1  chosen=0  depth=8,8,7,6,5,4,3,2,1,0  complete=false×7日
10s  設定: day=0..9 すべて pool=1  chosen=0  depth=8..0                     complete=false×7日
4M   設定: day=0..9 すべて pool=1  chosen=0  depth=8..0                     complete=false×8日
```

- **pool==1 = 「日次ラベル探索が 1 つも解を出せず、埋め込み fleet の計画だけがプールに入った」日**。
  `decide()` は pool.len()==1 のときビームを回しても比較相手がいないため chosen=0（= fleet の計画）をそのまま返す。
- つまり s5020 では **10日全部で convoy の label search / team 最適化 / ビーム比較が完全に不発**。
  width・depth・labels の感度はこの構造では意味を持たない（実測どおり B は 665〜672 のノイズ帯）。
- 「complete=false × 7日」はビームが depth 分の先読みを終えていないことの表れだが、
  これは pool==1 の子は 1 ノードしかないので打ち切りではなく、**ノード処理自体が各日 deadline で止まっている**ことによる。
- ラベル cap（1.5M）と deadline（budget_ms/3）の両方が切れている。時計なし（cap のみ）でも 1 日 60s 超で終わらず
  harness timeout → **cap 解放だけでは 32×32 の 1 日探索は完了しない**（4M cap + 59s でも pool==1 のまま）。

## 3. 日別分解（20s convoy vs 10s fleet、同一シミュレーション経路）

| day | convoy B日 | fleet B日 | 差 | 備考 |
|---|---|---|---|---|
| 0 | 62 | 61 | +1 | 双方 27 系列を当日確保（D=27 は day0 で確定） |
| 1 | 66 | 67 | −1 | |
| 2 | 79 | 79 | 0 | |
| 3 | 54 | 55 | −1 | 最短日 (66 step) |
| 4 | 76 | 76 | 0 | |
| 5 | 64 | 64 | 0 | |
| 6 | 71 | 71 | 0 | |
| 7 | 61 | 62 | −1 | |
| 8 | 69 | 69 | 0 | |
| 9 | 70 | 70 | 0 | |
| 計 | **672** | **674** | **−2** | 揺れ込み。V/D 完全同点 |

- D（=27 系列/日）は day0 に到達可能な系列が 27/27 で尽きる盤面のため、**B 以外の競争軸が存在しない**。
  fleet も convoy（=fleet プラン）も毎日全系列を回収し、B は在庫取り合いの端数のみで決まる。
- 日別差 ±1〜2 は「埋め込み fleet（budget_ms/4=5s、chains=1 相当）」と「本物 fleet（10s、chains=コア数）」の
  探索時間差に由来。**日程間の割当損失は観測されない**（どちらも毎日在庫をほぼ使い切る）。

## 4. 原因の確定

(a) **探索打ち切り（ラベル爆発）** — これが本体。
- 32×32 = 1024 セル × 28 スポット × steps 66..102 のラベル探索は、cap 1.5M どころか 4M でも 1 日分が終わらない。
  `patrol_day()`（`convoy_solver.rs:371`）が None を返し、`day_plans()`（:792）が即座に fleet プランのみで返る。
- pool==1 のとき convoy は 1 日あたり budget の 1/3 を label search の試行に費やした後、
  残り 2/3 を「子 1 個のビーム」に無駄に使う。**探索時間の実質 100% が捨てられている**。

(b) ビーム選択 — 無関係（pool==1 で比較対象が存在しないため発火しない）。

(c) 固有の計画質の差 — なし（convoy が返すのは fleet の計画そのもの。B 差は探索時間差のノイズ）。

#426 の「4f で日次上界に対し 0.2〜0.3 D 下」という所見は 16×16 の話であり、
32×32 では日次の質以前の問題（探索が走っていない）。

## 5. 最有力な修正（24×24 / 32×32 での B 回収見積もり）

**ラベル空間の再設計が唯一の修正**。優先順:

1. **マスクの系列クラスタ分割**（#426 調査項目 4 の「系列クラスタ分割」）:
   27 系列を地理クラスタ（例: 象限ごと 3〜4 クラスタ）に分割し、1 巡回車の担当クラスタだけを
   マスクに持たせる。マスク次元を実質 2^7〜2^8 相当に落とせばラベル数は 10²〜10³ 分の 1。
   期待回収: 現状の「convoy が動かない失点ゼロ」に加え、**24×24 の B−4..−7 も同じ構造で消える見込み**
   （24×24=576 セルでも 28 スポット超えやすい cap は同じ）。
2. **cap をスポット数に比例させるだけでは足りない**（4M で不発を実証済み）。cap 段階解放は単独では効果なし。
3. **pool==1 検知時の早期撤退**: label search が最初の patrol で None を返したら、残り 2/3 の予算を
   埋め込み fleet に渡す（現状は捨てている）。これは再設計までの繋ぎで、B は動かないが
   **応答 20s→5s**（ランキング軸 4 の無駄を削る）。
4. 32 スポット超 fallback（`Board::new` の spots>32 で None）は本盤面では未発火（28 スポット）。
   24×24/32×32 でスポット 33 個以上の盤面が来たら即 fallback になる点は別途対応（クラスタ分割で同時に解決）。

**B 回収の期待値**: s5020 では fleet と同点が現状の到達点（B −2〜0）。convoy の探索が動けば
合流・エスコート構造により 16×16 で +3〜9 B 上回った前例（round1 §6）から、24×24 で +4〜7、
32×32 でも +数 B の回収余地がある。ただし s5020 は D=27 が day0 で飽和する特異盤面で、
B の上振れは在庫 7〜8 の高在庫スポットへの送客数次第（在庫 8×2 系列など）。

## 6. 留意点

- 本測定の convoy は handoff 設定（budget_ms=20000）。#426 の −37 実測は 10s 設定・負荷の違う環境。
  いずれにせよ pool==1 構造は設定非依存（10s/20s/59s、1.5M/4M の全組合せで pool==1）。
- `trackb_diag.rs`（未コミット）+ `tracing-core` 依存 1 行が本 worktree に残っている。
  コミットは Supervisor 判断。診断 JSONL と trace は `/tmp/trackb-s5020/` 配下。
- 実戦は 1S ゲートによりこの盤面は fleet 投入。convoy の修正価値は 2S 以上の 24×24/32×32 盤面。

## 生データ

- `/tmp/trackb-s5020/fleet-10s/runs.jsonl` — fleet 10s（B679）
- `/tmp/trackb-s5020/convoy-20s/runs.jsonl` — convoy 20s ×2（B672）
- `/tmp/trackb-s5020/convoy-labels4m/runs.jsonl` — labels=4M（B669）
- `/tmp/trackb-s5020/convoy-w48d12/runs.jsonl` — width=48 depth=12（B665）
- `/tmp/trackb-s5020/convoy-30s/runs.jsonl` — 30s（B670）
- `/tmp/trackb-s5020/convoy-59s-4m/runs.jsonl` — 59s+4M（B671）
- `/tmp/trackb-s5020/convoy-noclock/runs.jsonl` — 時計なし（timeout 605s）
- `/tmp/trackb-s5020/{convoy,fleet}-diag.jsonl` + `*-trace.log` — 日別診断 + tracing
