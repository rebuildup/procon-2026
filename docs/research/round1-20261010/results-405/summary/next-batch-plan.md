# 次バッチ計画（F3-clean / F3-clocked / E3 / D ラダー）

2026-10-11 作成（バッチ設計・測定未実行）。作業 worktree `.worktrees/eval-wsl-405`（branch eval-convoy-415、HEAD fuel_need/fuel_mix 取り込み済み）、binary `target/release/procon-eval` sha256 **`d5eb038d…`**（source 2ed7906）。実行前に `sha256sum target/release/procon-eval` で一致確認をすること。

前提条件（全バッチ共通）: **静穏時実行**（§5）。kinds は全バッチ `S,P,P,P,P,P,P,P`、nice 15、jobs 2。出力先は `results-405/` 配下の新規ディレクトリ。コーパス盤面のベースパス: `docs/evidence/campaign-311/fleet-371/`（以下 `<CORPUS>` と略記）。

判定ルールは `param-optimization-plan.md` の冒頭規則を継承: 採用判定は公式辞書順 (V → dV → B)、B 差 ±20 未満はノイズ、clocked は可能なら seed 反転 2 実行、D 非劣化を必ず監視。

---

## 1. fuel/day0-step 比の検証結果（§6.1 照合 — 完了）

§6.1 の記載値を 36 JSON（`fuelLimits` ÷ `daySteps[0]`）で再計算した結果、**不一致は 1 件もない**:

- hard 12 盤面 = すべて比 1.0（`fuelLimits` == `daySteps[0]`、short 型）— 一致
- hold 24 盤面 = 1.0–3.0 — 一致
- mixed 型で比 1.0 は 3 件（s5001 49/49, s5009 171/171, s5017 216/216）— 一致
- short 型 = 1.268–3.0（記載「1.27–3.0」）— 一致
- スクリーニング 9: s9003/s9005/s9008/s9012/s5001 = 1.0、s5023 = 1.378（記載 1.38）、s5016 = 2.721（2.72）、s5013 = 2.777（2.78）、s5020 = 3.0 — すべて一致

境界注意: **s5011 = 1.500**（≤1.5 側の境界）、**s5006 = 2.000**（≥2 側の境界）。条件付き選択枠（§3 fuel 軸）の検証でこの 2 盤面は境界ケースとして別扱いにすること。

## 2. 全 36 盤面の fuel/day0-step 比表（昇順）

| 比 | grp | case | 型 | fuel/day0 | 区分 |
|---|---|---|---|---|---|
| 1.000 | hard | s9001 | short | 36/36 | ≤1.5 |
| 1.000 | hard | s9002 | short | 35/35 | ≤1.5 |
| 1.000 | hard | s9003 | short | 32/32 | ≤1.5 |
| 1.000 | hard | s9004 | short | 35/35 | ≤1.5 |
| 1.000 | hard | s9005 | short | 52/52 | ≤1.5 |
| 1.000 | hard | s9006 | short | 52/52 | ≤1.5 |
| 1.000 | hard | s9007 | short | 50/50 | ≤1.5 |
| 1.000 | hard | s9008 | short | 50/50 | ≤1.5 |
| 1.000 | hard | s9009 | short | 73/73 | ≤1.5 |
| 1.000 | hard | s9010 | short | 73/73 | ≤1.5 |
| 1.000 | hard | s9011 | short | 68/68 | ≤1.5 |
| 1.000 | hard | s9012 | short | 70/70 | ≤1.5 |
| 1.000 | hold | s5001 | mixed | 49/49 | ≤1.5 |
| 1.000 | hold | s5009 | mixed | 171/171 | ≤1.5 |
| 1.000 | hold | s5017 | mixed | 216/216 | ≤1.5 |
| 1.268 | hold | s5008 | short | 52/41 | ≤1.5 |
| 1.378 | hold | s5023 | mixed | 102/74 | ≤1.5 |
| 1.421 | hold | s5014 | short | 81/57 | ≤1.5 |
| 1.494 | hold | s5003 | mixed | 133/89 | ≤1.5 |
| 1.495 | hold | s5019 | mixed | 136/91 | ≤1.5 |
| 1.500 | hold | s5011 | mixed | 99/66 | ≤1.5（境界） |
| 1.682 | hold | s5021 | mixed | 180/107 | 1.5–2 |
| 1.910 | hold | s5005 | mixed | 128/67 | 1.5–2 |
| 2.000 | hold | s5006 | short | 96/48 | ≥2（境界） |
| 2.198 | hold | s5024 | short | 189/86 | ≥2 |
| 2.347 | hold | s5022 | short | 169/72 | ≥2 |
| 2.394 | hold | s5002 | short | 79/33 | ≥2 |
| 2.395 | hold | s5018 | short | 194/81 | ≥2 |
| 2.400 | hold | s5010 | short | 144/60 | ≥2 |
| 2.574 | hold | s5007 | mixed | 278/108 | ≥2 |
| 2.721 | hold | s5016 | short | 166/61 | ≥2 |
| 2.777 | hold | s5013 | mixed | 261/94 | ≥2 |
| 2.892 | hold | s5015 | mixed | 240/83 | ≥2 |
| 3.000 | hold | s5004 | short | 144/48 | ≥2 |
| 3.000 | hold | s5012 | short | 165/55 | ≥2 |
| 3.000 | hold | s5020 | short | 246/82 | ≥2 |

区分集計: ≤1.5 = 21 盤面（hard 12 + hold 9）/ 1.5–2 = 2（s5021, s5005）/ ≥2 = 13。fuel 軸の条件付き選択仮枠「比 ≥2 → fuel_need 不使用・比 ≤1.5 → 有効」の検証母集団としては、スクリーニング 9 に比 ≥2 が 3 件（s5016/s5013/s5020）しか入っていないため、**F3-clocked の判定は仮枠の一次確認に留まり、確定には検証 27 盤面での F 系追加（≤1.5 9 盤面・≥2 13 盤面からの追加サンプリング）が要る**。

---

## 3. バッチ草案

### バッチ A: F3-clean 再実行（offline 決定論）

- **目的**: `wsl-corpus/f3-offline-fuel/`（2026-10-10 23:20–00:1x 実行・19/108 セルが負荷起因 invalid）のうち、スクリーニング 9 盤面の invalid セルを scored で再取得し、真欠落の s5020 c8 fuel/mix を補完する。決定論なので既知 success セルの (V,dV,B) は再現するはずで、**一致確認が本目的**（バイナリ同一性確認の代替でもある）。
- **盤面（9・スクリーニング set）**: `<CORPUS>/corpus-hard/hard-16-a3-d9-short-s9003.json`、`hard-24-a5-d7-short-s9005.json`、`hard-24-a6-d9-short-s9008.json`、`hard-32-a8-d7-short-s9012.json`、`<CORPUS>/corpus-holdout/hold-16-a4-d7-mixed-s5001.json`、`hold-24-a5-d4-mixed-s5013.json`、`hold-24-a6-d10-short-s5016.json`、`hold-32-a6-d6-mixed-s5023.json`、`hold-32-a8-d10-short-s5020.json`
- **候補（6・clockless）**:
  `fleet:budget=official:iters=60000:seed=42:chains=4` /
  `…chains=4:fuel_need=50` / `…chains=4:fuel_need=50:fuel_mix=on` /
  `…chains=8` / `…chains=8:fuel_need=50` / `…chains=8:fuel_need=50:fuel_mix=on`
- **jobs 2**・54 runs
- **予想ウォール**: 25–40 分（E1 offline 実績: 45 runs ≈6 分 @jobs 6。スクリーニング 9 は重い盤面で rt 40–175s/run、jobs 2 で換算）
- **コマンド草案**:
  ```
  nice -n 15 target/release/procon-eval --quiet \
    --cases all --case-files <上記 9 パス csv> --kinds 'S,P,P,P,P,P,P,P' \
    --candidates '<6 候補 csv>' --jobs 2 \
    --output-dir docs/research/round1-20261010/results-405/wsl-corpus/f3-clean-fuel
  ```
- **判定基準**: ①全 54 runs success・scored（invalid 0）②既知セルの (V,dV,B) が `f3-offline-fuel/runs.jsonl` の scored 値と**完全一致**（1 セルでも不一致ならバイナリ差異を疑い、以降のバッチを停止して報告）③s5020 c8 fuel/mix の 2 セルに scored 値が初記録される。合計利得の確定値（fuel-only / mix、chain 別）を `fseries_gain.py` で再集計して progress ログに追記。

### バッチ B: F3-clocked（chains8 live の fuel 効果 — 採用判定の本丸）

- **目的**: clocked `budget_ms=10000:adapt=off` での fuel_need=50/fuel_mix=on 効果の確認。§6.2 の分解式 `gain(chains=c, mix=on) − gain(chains=c, mix=off)` を c=4/8 で clocked 側でも取り、offline 結論（mix > fuel-only・重複なし・D 下落 0）が live でも成立するかを判定する。chains8 採用リスク低減（s9008 offline D-loss 回復）の live 側裏取り。
- **盤面・候補数**: バッチ A と同一 9 盤面 × 6 候補・54 runs。候補は clocked 形式:
  `fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=4` /
  `…:chains=4:fuel_need=50` / `…:chains=4:fuel_need=50:fuel_mix=on` /
  `…:chains=8` / `…:chains=8:fuel_need=50` / `…:chains=8:fuel_need=50:fuel_mix=on`
- **jobs 2**・nice 15
- **予想ウォール**: 35–50 分（b3-20s の fleet@10s rt 実績 40–100s/run。10 日盤面が上限側、54 runs @jobs 2）
- **コマンド草案**: バッチ A と同形式。output-dir は `results-405/wsl-10s/f3-clocked-fuel`
- **判定基準**: ①全 54 runs success・illegal/fuel_short/rejected/timeout = 0（maxday は 10.0–10.2s 想定、60s 期限に余裕）②§6.2 分解式の clocked 版判定（c4 と c8 で mix 利得が同幅 = 独立 / 拡大 = 相乗 / 縮小 = 希釈）③**比 ≥2 の 3 盤面（s5016 2.72 / s5013 2.78 / s5020 3.0）で利得 ≈0（|ΔB| < ノイズ ±20）かつ比 ≤1.4 盤面に利得が集中** → §6.1 の仮説が clocked でも成立 ④dV 下落セル 0（mode (b) 判定・chains4/chains8 別々に確認）。dV 下落や比 ≥2 での B 大幅変動があれば条件付き選択枠の見直し（採用推薦は検証 27 盤面確認まで保留のまま）。
- seed 反転: 余裕があれば `seed=7` で c4f/c8f の 2 候補のみ追加（+18 runs ≈ +15 分）。必須ではない（offline で決定論裏取り済みのため）。

### バッチ C: E3 予算カーブ（fleet clocked budget ラダー）

- **目的**: fleet clocked の budget_ms ラダー {2500, 5000, 10000, 20000} × chains {4, 8} を同一実行内アンカーで測定し、**B が飽和する予算点**と axis4（response_time）とのトレードオフを確定する。S2（embedded fleet 予算比率のハードコード）分析の P1 補完（`p1-embed-rung` の 2.5s/5s 単点をラダー化）でもある。
- **盤面（4）**: `<CORPUS>/corpus-hard/hard-32-a8-d7-short-s9012.json`（32×32・比 1.0）、`hard-24-a6-d9-short-s9008.json`（24×24）、`<CORPUS>/corpus-holdout/hold-32-a8-d10-short-s5020.json`（32×32・比 3.0）、`hold-32-a6-d6-mixed-s5023.json`（32×32・比 1.38）
- **候補（8）**: `fleet:budget=official:budget_ms={2500,5000,10000,20000}:adapt=off:seed=42:chains=4` と同 `chains=8` の直積。`budget_ms=10000:chains=4` がアンカー兼ラダー点。32 runs・jobs 2
- **予想ウォール**: 20–35 分（rt ≈ 日数 × budget + オーバーヘッド。7 日盤面で 2.5s→~25s / 20s→~110s、10 日盤面の 20s は ~150s。4 盤面 × 8 候補 = 総 rt ~1500–2500s を jobs 2 で消化）
- **コマンド草案**: 同形式。output-dir は `results-405/wsl-10s/e3-budget-ladder`
- **判定基準**: ①全 32 runs success（daily_response_us が budget_ms×1.05 を超える日があれば列挙）②chain 階層ごとに budget ラダーの B 推移を見て、隣接 rung 差がノイズ（±20）以下に落ちる最小 budget を「飽和点」として報告（chains4/chains8 別）③chains8 利得の budget 依存（小予算で chains8 が chains4 に並ぶか・budget と chains の交互作用）④axis4 とのトレードオフ表: B 利得 vs `budget_ms`（20s は axis4 コスト 2 倍で辭書順での採用可否は別途 §4 プロトコル）⑤s5020 は比 3.0 盤面なので fuel 軸中立の盤面として予算カーブの対照群にも使う。

### バッチ D: convoy depth ラダー（corpus 未測定の補完）

- **目的**: §1.2 の「corpus での depth ラダー未測定」を補う。match3（16×16）では depth≥1 必須・2–3 で飽和・既定 8 と同等という結果があるため、24×24 corpus でも飽和するかを確認する。depth を下げられた盤面では beam 時間余剰が生じる（S3 の適応化の前提値）。
- **盤面（2・24×24）**: `<CORPUS>/corpus-hard/hard-24-a6-d9-short-s9008.json`（chains8 との比較で D が動く盤面）、`hard-24-a5-d7-short-s9005.json`
- **候補（3）**: `convoy:budget=official:budget_ms=10000:seed=42:depth=1` / `…depth=2` / `…depth=8`（8 は既定だが明示）。depth=8 がアンカー。6 runs・jobs 2
- **予想ウォール**: 10–20 分（convoy@10s rt 実績 ~40–105s/run）
- **コマンド草案**: 同形式。output-dir は `results-405/wsl-10s/d-depth-ladder`
- **判定基準**: ①全 6 runs success・illegal/fuel_short/timeout 0（day0 オーバーラン +0.9–1.0s の既知実績を監視）②(V,dV,B) を depth 1/2/8 で比較: match3 結果（d2 で飽和）と同じなら depth=2 が depth=8 に辞書順非劣化 → S3 適応化の根拠値。**dV が depth で動く場合は即座に報告**（24×24 では chains8/convoy 選択が dV 境界上にあり、depth も選択キーに影響しうる）③s9008 の dV は clocked chains8 基準 180（b4-chains8-full では 179）に対して劣化しないこと。

---

## 4. 実行順序推奨

1. **バッチ A（F3-clean）** — 最優先。決定論・負荷非依存だが他セッションがあると wall 超過で invalid になるため静穏時必須。F 系列の合計利得を確定させてから次に進む（s5020 c8 が欠けたままでは §6.2 の合計判定が確定しない）。
2. **バッチ B（F3-clocked）** — 採用判定の本丸。A の一致確認が済んだ同一バイナリで実行。
3. **バッチ D（depth ラダー）** — 最小バッチ（~15 分）。§3 の clocked 24×24 選択枠の再評価に直結するため C より先に結果が欲しい。
4. **バッチ C（E3 予算カーブ）** — セル数最多・ウォール最長。S2 分析の補完なので D と入れ替えてもよい（D 完了後なら日内に収まる）。

合計ウォール見積: ~1.5–2.5 時間（seed7 追加分を含めて上限）。各バッチ完了後に `runs.jsonl` + `aggregate.json` を証跡として確定させ、progress ログへ 1 エントリ追記してから次バッチへ移る（中断時のリカバリ点）。

## 5. 静穏判定条件（各バッチ開始前に毎回確認）

```bash
cat /proc/loadavg            # 第 1 フィールド < 4 であること
pgrep -af procon-eval        # 出力 0 行であること（他セッションの eval が走っていない）
pgrep -af 'target/release'   # 出力 0 行であること（他 worktree のビルド/計測プロセス）
```

- F3-offline の教訓（3 セッション並走で load 20・19/108 セル invalid）から、**開始前確認だけでなく開始 5 分後にも再確認**し、load が 8 を超えたら一旦停止（証跡未書き出しのまま停止してよい — 決定論バッチは再実行で回収可能）して原因セッションの完了を待つ。
- fleet clocked は worker あたりコア数スレッドを張るため、並走 eval と相互汚染する。convoy（バッチ D）はbeam/label search が単スレッド主体だが deadline オーバーランが既知のため、同じく静穏時を前提とする。
- 測定の jobs は 2 で固定（b4-chains8-full の jobs 3 では B 変動が観測されたため、同一実行内比較に影響しないバッチ C でも 2 を維持）。