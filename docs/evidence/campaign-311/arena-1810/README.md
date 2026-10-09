# Arena match 1810 — 7台 coverage park=far が tick 修正統合後 SHA で完走

外部アリーナ（`https://procon37arena.online`）match **1810**（2026-10-09 10:30 JST 窓）。
**7 エージェント構成の `coverage:budget=official:park=far` が revision=-1 なしで completed** —
coverage 系で初の 7 台完走。優先課題「5台以上の plan rejection」（#317 / #331）に対する
**修正統合後 SHA での初の live 検証**であり、仮説を支持する結果となった。

- source: `60ed527db5cdfbd706cc3d816e88ff43c3295a44`（origin/331 系。`8728ed5` + `2c5800d` + `6ff04c1` + `e37c5b0` を含む）
- 実行: 外部アリーナ専任セッション（single-operator。投入依頼は #317 に記録）
- 台帳: #326。関連: #317（選抜）、PR #332（候補本体）、#311（親）

## 試合結果（§5 形式）

| 項目 | 値 |
|---|---|
| Match ID / 実行日時 | **1810** / 2026-10-09 10:30:00 JST（登録 10:27:10、記録 10:36:05） |
| Candidate / SHA | `coverage:budget=official:park=far` / `60ed527` |
| agent kinds / 台数 | `P,S,P,S,P,S,P`（既定交替）/ **7 台**（4 Patrol + 3 Supply） |
| 判定 | **completed**（拒否なし、error null、不正行動の兆候なし） |
| 全日の revision | **[3, 23, 58, 93, 127]**（全日受理・単調増加） |
| days_completed | **5/5**（daySteps [72, 75, 95, 78, 93]、daySeconds 全日 60） |
| 公式スコア / 順位 | **未取得** — `result_ref: null`（OpenAPI に score/rank 端点なし）。arena Web UI からの人手記録待ち |
| 燃料 | 初期 145。**agent#6（Patrol）のみ 145→90→13→0 と枯渇**（day3 以降は全日待機）。他 6 台は全日 145 で健全 |
| 不正行動 / 応答遅延 | ログに reject / invalid シグナルなし。day0 提出は departure+124s で受理（下記「観察」） |
| 実行ログ | `run-1030.log`（RUST_LOG=debug、提出 body 全文。token 検出 0 件を確認済み） |
| 再現手順 | 「再現手順」節 |

## 検証仮説と結果（優先事項 2）

**仮説**: 過去の coverage 5+ 台拒否（1760 / 1796 / 1798 / 1800）は、公式 tick 補給修正
`8728ed5`（PR #324）が統合**以前**の SHA で実行されたためであり、修正統合済み SHA では
5+ 台でも revision=-1 が再現しない。

**根拠（投入前の ancestry 検証）**: `8728ed5` の commit 時刻は 2026-10-09 06:52 JST、
PR #324 の merge（`3549886`）は 09:48 JST。`git merge-base --is-ancestor` で
`8728ed5` は `6ff04c1`（01:55）・`2c5800d`（08:42）の祖先に**ならない** —
つまり過去の coverage 拒否試合は**誰も tick 修正を含んでいない**。

| match | 時刻 (JST) | SHA | 台数 | tick修正 `8728ed5` | 結果 |
|---|---|---|---|---|---|
| 1760 | 10/09 03:30 | `6ff04c1` | 7 | ✗ | rejected day=0 |
| 1796 | 10/09 08:30 | `6ff04c1` | 5 | ✗ | rejected day=3 |
| 1798 | 10/09 08:45 | `2c5800d` | 7 | ✗ | rejected day=1（重複投入で contaminated、下記） |
| 1800 | 10/09 09:00 | `2c5800d` | ? | ✗ | rejected day=8 |
| **1810** | **10/09 10:30** | **`60ed527`** | **7** | **✓** | **completed 5/5** |

**結果: 仮説を支持**（`2c5800d` 単独では 1798/1800 が拒否されており、差分は
`8728ed5` の有無）。ただし 1 試合であり、マップ・台数は試合ごとに異なるため
因果の完全証明ではない。継続投入で再現性を確認する。

## 投入前の分析（気づきの記録）

1. **過去の全 coverage 拒否は修正以前の SHA**（上表の通り）。修正統合後の coverage
   投入は本試合が初回。
2. **重複投入の実例 — match 1798**: 08:45 窓に release030 worktree（search @`71724bf`）と
   campaign-331（coverage @`2c5800d`）の 2 セッションが同一 team token で同時投入し、
   両 evidence が同一 match_id 1798 / 同一 window を記録。当該 window の両拒否記録は
   contaminated として扱う。以後は本セッションが外部アリーナを single-operator で専有。
3. **wire_plan の tick 修正が 2 系統に分岐**: `8728ed5`（release-0-4-0 / PR #324 系）と
   `5de53d9`（branch 315 系）は `wire_plan.rs` の結果内容が異なる（233+/169−）。
   search @`5de53d9` は 5 台 completed 実績（1787）。両系統の調和は開発側の判断事項。
4. **#326 台帳 rows 9-10 の時刻ずれ**: 台帳の 07:30/07:45 に対し evidence の
   window_unix は 08:30/08:45 JST（rows 1-8 は一致）。
5. **PR #332 head `9183d81` はコンパイル不能**: merge commit（parents `e37c5b0` +
   `3549886`）で `search_solver.rs` の `CoverageCandidate { variant: 0 }` 初期化が
   `park_risk` フィールドに未追従（E0063 ×3）。`cargo build --release --locked
   --bin procon` が失敗するため、投入 SHA を origin/331 tip から
   origin/333 tip `60ed527`（同一統合内容・初期化一貫）へ変更した（#317 に記録）。
6. **`just` が mise shim 未設定で動作しない**: `mise ERROR No version is set for
   shim: just`。arena.md の等価直接起動（`cargo run … coordinator --arena --next`）で代替。

## 試合内の観察（開発へのフィードバック）

- **燃料枯渇**: agent#6（ Patrol、7 台目）のみ fuel が 145→90→13→0（day3 始で枯渇）。
  day2 に moves 8 / waits 84 へ落ち、day3-4 は全日待機。3 Supply でも 7 台中 1 台が
  給油圏外にいる構造。4 Patrol 構成の玉数天井（#311 checkpoint）とは別軸の損失。
- **day cadence**: 2 日目以降は 60 秒ぴったりで提出が進む（`plan generated` 間隔 = 60s）。
- **day0**: departure（10:30:01）から 124 秒後の 10:32:05 提出が revision=3 で受理。
  サーバ側の day0 回答窓は departure 直後ではない可能性がある（観察のみ。仕様断定はしない）。
- **registration**: lobby open（10:27:00）直後 1 発目は `no match is scheduled right now`、
  2 発目（+10s）で `lobby: you are registered … match 1810` — arena.md の成功パターン 2 と一致。
  登録後は poll 停止を確認（先着順規律どおり）。

## 再現手順

```bash
# 隔離 worktree で pinned SHA をビルド（just は mise 未設定のため直接起動）
git worktree add --detach .worktrees/arena-60ed527 60ed527
cp .infisical.json .worktrees/arena-60ed527/   # token は Infisical runtime 注入のみ
cd .worktrees/arena-60ed527
CARGO_TARGET_DIR=<shared>/target cargo build --release --locked --bin procon

# 次の cadence window まで待って 1 試合のみ（token を引数にしない）
PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" RUST_LOG=debug \
  infisical run --env=dev -- cargo run --release --locked --bin procon -- \
    coordinator --arena --next --candidate 'coverage:budget=official:park=far'
```

- 衝突確認: 投入前に coordinator プロセスを `ps` で確認し、#317 に予約コメントを残す。
- evidence: `target/arena/runs.jsonl`（本ディレクトリにコピー済み）。
- 試合中の heavy eval は実行しない（#326 運用規律）。

## ファイル

| ファイル | 内容 |
|---|---|
| `runs.jsonl` | evidence 1 行（`record_kind: "arena-run"`、schema は arena.md） |
| `run-1030.log` | RUST_LOG=debug の実行ログ（提出 body 全文・登録経緯。token 検出 0 件） |
| `source-sha.txt` | `60ed527db5cdfbd706cc3d816e88ff43c3295a44` |

## 未了

- [ ] rank / 4 段 score の人手記録（arena Web UI、match 1810）→ #326 台帳へ
- [ ] 仮説の再現性確認（次窗以降の 5+ 台投入）
- [ ] `9183d81` の build 修正（PR #332）、`8728ed5` / `5de53d9` の調和（開発側）
