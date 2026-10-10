# 2026-10-10 本選1回戦 — 実戦証跡・失点解析基準

親Issue: [#403](https://github.com/rebuildup/procon-2026/issues/403) / 初期データブランチ: `403` / 統合先: `release-0-5-1`。

## 0. 位置付けと証拠の格付け

この文書は解析タスクの**入口・再現規約**。この段階では「3位の原因」や「最良の改善戦略」を確定していない。本番ソルバの変更や次の試合への投入も行わない。

証拠には必ず次の `provenance` を付す。

| provenance | 意味 | 扱い |
|---|---|---|
| `official` | 運営の試合結果・スコアボード等、公式に確定した数値 | 最終順位/スコアの正本 |
| `event_display_screenshot` | 会場の順位発表画面を撮影した画像・その転記 | 画面上の順位・V/D/B/時間の証拠。API原本や全チーム記録ではない |
| `user_report` | 人手による順位等の報告 | スクリーンショット等の証跡がない場合は暫定 |
| `live_observed` | `events.jsonl`、`coordinator.log`、`state.json` 等に記録された通信・観測・提出結果 | 記録の時点/範囲に限った実測 |
| `engine_replay` | 指定した盤面・初期条件・候補を公式Rust評価器/MatchSimで再生した値 | 実戦の10人交通を再現できない場合は別スコア |
| `model` | ソルバ内部の `model_*` / `replay_*` / `tomorrow_*` 予測・近似 | 公式採取実績を示すものではない |
| `estimate` | 観測外の行動や交通を仮定した推計・反実仮想 | 必ず仮定と上下界を記す |

欠けたデータは `null / N/A` とし、0点や推定値で埋めない。**後日追加の順位発表スクリーンショットで、第3試合の宇部3位 / V10・D44・B51・画面表示時間00:54を確認した**。ただし順位画像は各試合上位6名までで、公式API原本と全10チームの記録はない。

## 1. 入力（原本は変更不可）

`data/round1/20261010T044644Z-p28796/` に以下を原文のまま置いた。

| ファイル | SHA256 (原本) | 用途 |
|---|---|---|
| `coordinator.log` | `f0431e2499c98491fec2ec30abf83a2b7d0143aa6bb083792690e000d843754e` | planner選択・各日結果のログ |
| `events.jsonl` | `549075c9f66592ee8163308de1cca06a0fc094328252b8fac42a61d95729efb4` | 時系列のkind/state/plan/submit/match_endイベント |
| `manifest.json` | `5446d77bbea52bb521b20df778380e712633f9a1681a710edc882630b67a2681` | ビルド同定・収録情報 |
| `setting.json` | `5478dae2b36c00fc420c4e7935a1ab2dd3065a2fe915a04d5ab174d02d853fe4` | サーバーから取得した盤面 |
| `state.json` | `536759361c5b4db38875f7dcc644d7a0c8c7c939e59eb760d6b596645304e2bc` | match_end の状態 |

`baseline.json` は原本ログと後日取得の順位画像を統合した**派生サマリー**であり原本ではない。画像とその転記・原本PNG SHA256・Git blob SHAは `screenshots/` と `scoreboards.json` にある。新たな証跡・集計・counterfactualは別ファイルを作る。

## 2. 初期観測の固定点

- run ID: `20261010T044644Z-p28796` / ソース: `7a93ad76549397e21178476d472a6a070ecddfce` / バージョン: `0.5.0`
- Windows x86_64 実行binary SHA256: `ff28c73a8c43fbfba2cdb3339dc7c37e8cfbfc6c03bdd46d2e1818cc06d636cf`。ソースSHAは manifest の `git-working-directory` 由来であり、clean tree を証明しない点に注意。
- Candidate: `fleet:budget=official:budget_ms=10000:adapt=off`。車種自動選択で `S,P,P,P`（補給1・巡回3）。
- 16×16・4台・10名・5日・10系列・16スポット・燃料40・daySteps `32,36,40,40,48`・daySeconds各60。
- `events.jsonl` は25イベント・欠落0・書込エラー0。**5日全て accepted**、各日1回で提出成功、最終 `completed`。
- Day 0〜4 の回答時間ms: `10004, 10004, 10004, 10003, 10005`。余裕ms: `50603, 49589, 49571, 49559, 49539`。
- `GET /state` の `others` と `traffics` は各日の観測スナップショット。全チームの完全な行動時系列ではない。
- `manifest.json` は `logs/20261010T044644Z-p28796/http.jsonl` を参照しているが**今回の5ログファイルには含まれない**。順位画像は別途 `screenshots/` に追加済み。

## 3. 得点の評価方法

公式比較は **V（総種類数）→ D（日別種類数合計）→ B（総玉数）→ 応答時間が短い方**の辞書順。前段で負ける改善は後段で挽回できない。時間・回答期限・合法性/受理は別のhard gateとして測る。

追加された上位6名の順位画像を入力として、最初に次を出す。

1. **第3試合**の上位1〜3位の画像転記 `V,D,B,T` と自分との差。第1・第2試合の上位6名は参考資料として分離。
2. Day 0〜4 の `V_day,D_day,B_day`、欠落系列、未採取スポット/在庫、余り歩数、期限・fuel消費。
3. agent別の移動・待機・補給・回収、日末の位置/燃料と翌日の損失。plan予測と実行観測を分離。
4. 「当日の取りこぼし」「翌日位置取り」「燃料/補給」「重複訪問」「他者交通」「モデルとlive差」に損失を分解。**根拠のない原因断定や、各仮説改善量の二重計上は禁止**。

## 4. 再現開始手順と制約

```bash
# データとソースの同定（branch 403 またはPR経由）
git rev-parse HEAD
sha256sum data/round1/20261010T044644Z-p28796/*
# 使えるCLI/評価器を現行ソースから再確認する
cargo run -p cli --bin procon -- --help
cargo run -p cli --bin procon -- match-config validate data/round1/20261010T044644Z-p28796/setting.json
```

- `setting.json` は生の公式設定。評価器が flat CaseSpec を要求するなら、別の**派生CaseSpec**を生成し原本と全設定値を照合（#398参照）。raw settingをそのまま `--case-files` に渡して成功したとみなさない。
- 出力先は `docs/research/round1-20261010/` や `data/round1/derived/` 等の原本と異なる場所。原本のhashが1文字でも異なれば解析中止・再確認。
- 実戦の人間以外の参加者行動は `day_state.observed.others` で見える範囲しか分からない。未知の交通はscenarioを分ける。後日の観測を当日開始時の既知情報として使う**未来情報リーク**は禁止。
- 1回戦への適合だけで採用しない。公式サンプル + 未使用盤面/holdoutで退化と提出期限の両方を確認する。
- ローカル解析のみ。公式サーバ・外部Arenaへの再接続や試合登録はしない。

## 5. エージェント成果物テンプレート

`analysis.md` を新設して次の順に記録する。

```text
Facts:    source, raw sha, score provenance, observed submission outcomes
Unknowns: missing machine-readable official results/HTTP logs/opponent decisions
Inference: hypothesis, evidence, uncertainty, alternative explanations
By-day:   day / V / D / B / agents / fuel / waits / unused spots / provenance
Gaps:     against official top teams (if observed) / estimable upper bounds
Trials:   sourceSHA, binarySHA, dataSHA, candidate, kind, budget, seed
          official-score-V/D/B/T, status, timeout/invalid/fuel_short,
          exact command, repeat dispersion, holdout W/D/L, negative findings
Decision: keep current, experiment further, or propose independent Candidate
```

**候補は独立したIssue・番号ブランチ・Draft PRで実装**。このブランチは根拠となる原本と解析規約の保存だけに限定する。改善の条件は公式4軸の勝ち、deadline遵守、他盤面非退化、ロールバック可能性。最適と確定できない場合は明記する。

関連: #403（親） / #398（予行盤） / #342（複数日・補給） / #373（上位チーム差） / #317（candidate選定）。

## 6. 1回戦の盤面画像と順位表（後日追加）

各試合について1枚の途中盤面と1枚の順位発表を保存。元の画像は2048×1151のPNGであり、リポジトリ内の版は容量を抑えた**同解像度のWebP（quality=88）**。元PNGのSHA256とWebPのGit blob SHAを `scoreboards.json` で管理する。

| 試合 | 途中の盤面 | 順位発表 | 撮影時点 |
|---|---|---|---|
| 第1試合 | [盤面](screenshots/match-1-map.webp) | [順位](screenshots/match-1-results.webp) | 2/5日目、21/32 Step |
| 第2試合 | [盤面](screenshots/match-2-map.webp) | [順位](screenshots/match-2-results.webp) | 0/5日目、タイプ選択中 |
| **第3試合** | [盤面](screenshots/match-3-map.webp) | [順位](screenshots/match-3-results.webp) | 1/5日目 |

[6チーム×3試合の順位表の構造化転記](scoreboards.json) はスクリーンショットを正本とする派生データ。転記ミスが見つかった場合は原画と照合して修正する。盤面画像は**途中時点の概略**で、正確な公式設定の代替ではない。

### 実戦runと一致するのは第3試合

`setting.json` の上端spot列（pos=1,5,9,15）・中央や左右の初期位置が第3試合盤面の画像と整合する。run ID `20261010T044644Z-p28796` の順位は**第3試合の「宇部」3位**と紐づける。第1試合の大分3位、第2試合の群馬3位と誤結合しない。

| 順位 | 第3試合の学校 | V | D | B | 回答時間（画面表示） |
|---:|---|---:|---:|---:|---|
| 1 | ハノイ国家大 | 10 | 44 | 57 | 04:31 |
| 2 | 新居浜 | 10 | 44 | 55 | 01:50 |
| **3** | **宇部** | **10** | **44** | **51** | **00:54** |
| 4 | 鶴岡 | 10 | 39 | 53 | 03:07 |
| 5 | 奈良 | 10 | 38 | 53 | 00:04 |
| 6 | 一関 | 0 | 0 | 0 | 05:00 |

### 優先検証仮説（上界ではなく順位境界の算術）

**今回の他チーム最終結果を固定した場合**、現状 V=10,D=44,B=51 に対し、Dを**+1（45）**できれば第2軸で優勝相当。Dが44のままでも Bを**+4（55）**なら新居浜とB同点・表示回答時間で上回り2位相当、**+6（57）**ならハノイ国家大とB同点・表示回答時間で上回り1位相当。時間も変わらないという条件付きの反実仮想であり、同条件で「回収できる」とは証明していない。

**分析優先度:** まずD欠落6（理論上限10種類×5日=50）について到達・補給・期限上の実現可能性を証明/反証し、次にDを維持して回収玉数を+4/+6以上増やせる余地を定量化する。V=10の保持が最上位制約。別試合（第2試合のD46等）とのスコア比較はあくまで他盤面での**参考**であり、同じ盤面での対戦差ではない。

`scoreboards.json` の**回答時間**は順位画面に表示された累積時間であり、`coordinator.log` の日別 `answer_ms` とは同義ではない。

