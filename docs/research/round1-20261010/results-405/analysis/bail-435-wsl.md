# PR #435 bail=on の WSL 計測結果（ラベル爆発盤面 4 枚・convoy@20s）

2026-10-11 00:16–00:34 JST。計測分析チーム（read-only + 本ファイルのみ新規作成）。
対象: PR #435（bail=on: convoy のラベル探索失敗時に残り予算を embedded fleet へ譲渡）。worktree `bail-435-verify`。

- source commit: `eb2f5de` (`eb2f5dee86fb55eca0e7f0fe6985faf6d4e45e35`、runs/aggregate 双方に記録)
- binary: `procon-eval` sha256 `85119c9f486e4635cdbfd4b97d2d25aeb41a9109ff9a629a05f481370ca11f66`
- 実行条件: `--jobs 1 --nice 19`、convoy:budget_ms=20000:seed=42、kinds `S,P,P,P,P,P,P,P`、同一実行内で parent と bail=on の 2 候補
- 出力: `.worktrees/bail-435-verify/eval-bail-435/{runs.jsonl (26 レコード), aggregate.json}`

**負荷注記**: 実行開始時の load average は 15.01 / 16.00 / 15.52（他セッションの eval p1-s7 計測と arena coordinator が並走）。rt は絶対値では高めに出ている可能性が高いが、**同一実行内の parent vs bail の相対比較**は両候補が同一負荷下で交互実行されるため有効。B 差の解釈は「**負荷ノイズ ±20 未満は有意としない**」ルールに従う（別実行の絶対比較は不可）。

---

## 1. 結論（先に）

**bail=on は「B 非劣化で axis-4（response time）を 4–6 倍改善する」。** ラベル爆発の 4 盤面すべてで V / D は同一、B は 2 勝（+57, +39）・1 分（−7, ノイズ内）・1 敗（−23）。parent では budget 20s の日が多々あり、ラベル探索の deadline 超過で最大 50s まで伸びていたのに対し、bail=on は全日 4.7–8.8s に収まる。違反指標（illegal / fuel_short / rejected / day_budget_exceeded）は全レコードで 0。

ただし **s9012 でのみ B が −23 劣化**（parent 387 → bail 364）。ノイズ閾値 ±20 を僅かに超えるため「有意な劣化の可能性あり」と扱う。採用判断はこの 1 盤面の要因確認を推奨（詳細 §4）。

---

## 2. 盤面別の辞書順判定（V → D → B → response_time）

| 盤面 | サイズ | parent V/D/B | bail V/D/B | 辞書順判定 | B 差 | 意味解釈 |
|---|---|---|---|---|---|---|
| hold-24-a6-d10-short-s5016 | 24×24 d10 | 20/200/**452** | 20/200/**445** | parent (B+7) | −7 | ±20 内 → 実質 draw |
| hold-32-a8-d10-short-s5020 | 32×32 d10 | 27/270/**615** | 27/270/**672** | **bail (B+57)** | +57 | 有意な bail 優位 |
| hold-32-a6-d6-mixed-s5023 | 32×32 d6 | 27/162/**413** | 27/162/**452** | **bail (B+39)** | +39 | 有意な bail 優位 |
| hard-32-a8-d7-short-s9012 | 32×32 d7 | 27/189/**387** | 27/189/**364** | **parent (B+23)** | −23 | 閾値僅か超過。劣化の可能性あり |

4 盤面すべて V / D が完全一致。B 合計差は **+66（bail 有利）**。status は全 8 セル `success`（scored=true）、illegal_action_count / fuel_short_count / rejected_response_count は 0、`day_budget_exceeded_days` は全盤面で空。

参考（別実行・絶対比較不可）: parent の s9012 B=387 は過去 b2-20s の 388 と整合し、今回の計測が正常レンジにあることを示す。

---

## 3. axis-4: response time の定量化（爆発盤面）

全日の response_time（秒、harness 計測値。負荷により絶対値は膨らむ方向にバイアス）:

| 盤面 | parent daily rt | bail daily rt | parent max | bail max | parent 総 rt | bail 総 rt |
|---|---|---|---|---|---|---|
| s5016 | 20.3–39.8（20s 張付多数） | 6.6–8.8 | **39.8s** | 8.8s | 219.6s | 72.2s |
| s5020 | 20.0–38.8 | 4.6–6.7 | **38.8s** | 6.7s | 211.2s | 54.7s |
| s5023 | 15.9–50.3 | 4.7–8.6 | **50.3s** | 8.6s | 143.8s | 36.5s |
| s9012 | 20.0–33.4 | 4.7–7.5 | **33.4s** | 7.5s | 145.8s | 45.6s |

観察事項:

1. **parent は budget 20s ちょうどに張り付く日が多い**（s5016 d2=39.8 / s5023 d2=50.3 / s5020 d7=38.8 など、ラベル探索が deadline チェック粒度（8192 labels ごと）を超えてオーバーランした形跡）。bail=on ではオーバーランが消失し、全日が 4.7–8.8s。
2. **期待値「20s → 5s」はほぼ達成**: bail の代表日 rt は 4.7–6.7s、最悪日でも 8.8s。残る ~2–4s は embedded fleet 自体の計画時間 + harness オーバーヘッド（高負荷下）で、負荷低減時はさらに縮む可能性。
3. 総 response time は 4 盤面で **3.0–4.1 倍短縮**（219.6→72.2 / 211.2→54.7 / 143.8→36.5 / 145.8→45.6）。
4. 高負荷の rt への影響は parent・bail 双方に等しく乗るため、「parent の 20s 張付き + オーバーラン → bail の ~5s」の構造差は負荷ノイズより 1 桁大きく、axis-4 改善の結論は頑健。

---

## 4. s9012 の B −23 の位置づけ

- 差は ±20 閾値を僅かに超える（+23）。ただし 1 試行・高負荷実行であり、再実行での反転余地はある。
- 仮説（今回のデータからは検証未了）: s9012 は parent でもラベル探索が部分的に完走できる盤面で、bail が pool[0]（embedded fleet）へ早期フォールバックすることで parent が beam 判定を通せた日と bail が fleet 計画で答えた日の差が出た可能性。
- s5016（24×24、parent 側も爆発が軽い）が −7 で済んでいることと併せると、劣化は「爆発の軽い盤面で beam の情報価値を失う」パターンと整合。s9012 は d7 で parent の daily rt が [20,20,20,23.8,33.4,21.6,6.7] と爆発日が限定されている点も整合的。

---

## 5. 同梱 fixture smoke（全 9 盤面）

全 17 fixture セル（openapi-sample / small-8x8 / medium-16x16 / large-32x32 / road-heavy / road-heavy-v2 / mountain-heavy / series-many / series-few）で `status: success`・違反 0。road-heavy-v2 のみ B 35 → 34（−1、小盤面のためノイズ扱い・池上 spot panic など異常なし）。全 9 盤面 smoke ゲートは通過。

---

## 6. 判定

- **辞書順**: 4 盤面で V / D 完全一致、B は bail 2 勝 / draw 1 / 敗 1。B 合計 +66。
- **B 非劣化**: s5016 はノイズ内、s5020 / s5023 は有意改善、**s9012 のみ −23 の僅かな劣化**。「B 非劣化」は平均的には成立するが盤面単位では完全ではない。
- **axis-4**: 全爆発盤面で max day rt が 33–50s → 6.7–8.8s、総 rt 3.0–4.1 倍短縮。期待効果（20s→5s）はほぼ達成。
- **安全性**: 違反指標すべて 0、全 smoke success。
- **結論**: bail=on は第 4 軸の改善効果が明確。残課題は s9012 の −23 の再現確認（低負荷での 1 再実行）。