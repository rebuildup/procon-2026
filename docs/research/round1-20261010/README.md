# 2026-10-10 本選1回戦 — 実戦証跡・失点解析基準

親Issue: [#403](https://github.com/rebuildup/procon-2026/issues/403) / 初期データブランチ: `403` / 統合先: `release-0-5-1`。

## 0. 位置付けと証拠の格付け

この文書は解析タスクの**入口・再現規約**。この段階では「3位の原因」や「最良の改善戦略」を確定していない。本番ソルバの変更や次の試合への投入も行わない。

証拠には必ず次の `provenance` を付す。

| provenance | 意味 | 扱い |
|---|---|---|
| `official` | 運営の試合結果・スコアボード等、公式に確定した数値 | 最終順位/スコアの正本 |
| `user_report` | 人手による順位等の報告 | 公式記録未受領の間は暫定 |
| `live_observed` | `events.jsonl`、`coordinator.log`、`state.json` 等に記録された通信・観測・提出結果 | 記録の時点/範囲に限った実測 |
| `engine_replay` | 指定した盤面・初期条件・候補を公式Rust評価器/MatchSimで再生した値 | 実戦の10人交通を再現できない場合は別スコア |
| `model` | ソルバ内部の `model_*` / `replay_*` / `tomorrow_*` 予測・近似 | 公式採取実績を示すものではない |
| `estimate` | 観測外の行動や交通を仮定した推計・反実仮想 | 必ず仮定と上下界を記す |

欠けたデータは `null / N/A` とし、0点や推定値で埋めない。全10チームの最終スコアや提出履歴は添付に含まれていない。**3位はユーザー報告**。

## 1. 入力（原本は変更不可）

`data/round1/20261010T044644Z-p28796/` に以下を原文のまま置いた。

| ファイル | SHA256 (原本) | 用途 |
|---|---|---|
| `coordinator.log` | `f0431e2499c98491fec2ec30abf83a2b7d0143aa6bb083792690e000d843754e` | planner選択・各日結果のログ |
| `events.jsonl` | `549075c9f66592ee8163308de1cca06a0fc094328252b8fac42a61d95729efb4` | 時系列のkind/state/plan/submit/match_endイベント |
| `manifest.json` | `5446d77bbea52bb521b20df778380e712633f9a1681a710edc882630b67a2681` | ビルド同定・収録情報 |
| `setting.json` | `5478dae2b36c00fc420c4e7935a1ab2dd3065a2fe915a04d5ab174d02d853fe4` | サーバーから取得した盤面 |
| `state.json` | `536759361c5b4db38875f7dcc644d7a0c8c7c939e59eb760d6b596645304e2bc` | match_end の状態 |

`baseline.json` はこのディレクトリから抜き出した**派生サマリー**であり原本ではない。新たな証跡・集計・counterfactualは別ファイルを作る。

## 2. 初期観測の固定点

- run ID: `20261010T044644Z-p28796` / ソース: `7a93ad76549397e21178476d472a6a070ecddfce` / バージョン: `0.5.0`
- Windows x86_64 実行binary SHA256: `ff28c73a8c43fbfba2cdb3339dc7c37e8cfbfc6c03bdd46d2e1818cc06d636cf`。ソースSHAは manifest の `git-working-directory` 由来であり、clean tree を証明しない点に注意。
- Candidate: `fleet:budget=official:budget_ms=10000:adapt=off`。車種自動選択で `S,P,P,P`（補給1・巡回3）。
- 16×16・4台・10名・5日・10系列・16スポット・燃料40・daySteps `32,36,40,40,48`・daySeconds各60。
- `events.jsonl` は25イベント・欠落0・書込エラー0。**5日全て accepted**、各日1回で提出成功、最終 `completed`。
- Day 0〜4 の回答時間ms: `10004, 10004, 10004, 10003, 10005`。余裕ms: `50603, 49589, 49571, 49559, 49539`。
- `GET /state` の `others` と `traffics` は各日の観測スナップショット。全チームの完全な行動時系列ではない。
- `manifest.json` は `logs/20261010T044644Z-p28796/http.jsonl` を参照しているが**今回の5ファイルには含まれない**。結果表も未提供。

## 3. 得点の評価方法

公式比較は **V（総種類数）→ D（日別種類数合計）→ B（総玉数）→ 応答時間が短い方**の辞書順。前段で負ける改善は後段で挽回できない。時間・回答期限・合法性/受理は別のhard gateとして測る。

データが得られたら、最初に次を出す。

1. 公式1〜3位の `V,D,B,T` と自分との差（同一試合の公式記録のみ）。
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
Unknowns: missing official scores/HTTP logs/opponent decisions
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
