# 非公式 arena 実行ガイド（Issue #179）

> 非公式練習場 `https://procon37arena.online` を **selected candidate の外部検証先**として
> 使うための実行・安全・記録のガイドです。
> 選抜方針と投入候補は Issue #179 の **shortlist comment**（supervisor 選抜 4 本）を参照。
> offline 側の評価経路は [docs/evaluation.md](../evaluation.md)（Issue #178）が唯一の canonical です。

**重要: 非公式 arena で成功しても公式互換性の完全な証明とは扱いません。**
本ドキュメントの実行は「他チーム/他 solver が存在する実環境での継続評価」のためにあり、
公式サーバ移植の保証・順位の保証・スコアの保証は一切しません。
逆に、公式互換性の根拠は常に公式 OpenAPI / 回答システム fixture 経由の offline 評価です。

## 二段構成

```text
offline（Issue #178, canonical）
  candidate sweep → engine::MatchSim → corpus ranking → selected candidates
                              ↓ 選抜した candidate id をそのまま渡す
online（本 Issue）
  selected candidate → procon37arena.online → 実プロトコル → evidence
```

- **arena は parameter sweep の実行先ではありません。** 大量探索はローカルで行い、
  arena には Issue #179 shortlist の 4 本のみを投入します
  （`multiloop:kmeans=4:seed=42` / `astar:policy=step` /
  `loop:iterations=32:initial=greedy:lookahead=crossday` /
  `multiloop:kmeans=4:seed=42:walk=astar`）。
- arena への投入も **既存の canonical coordinator 経路**
  （`protocol::MatchClient` + `run_coordinator_loop`）を使います。
  arena 専用の protocol client / duplicate coordinator は存在しません。

## 標準手順（token は CLI 引数にしない）

```bash
# 1. shortlist の candidate を Issue #179 comment から決める
# 2. Infisical から PROCON_ARENA_TOKEN を注入したまま起動する
infisical run --env=dev -- just arena-next --candidate 'multiloop:kmeans=4:seed=42'
```

等価な直接起動:

```bash
infisical run --env=dev -- cargo run --release --bin procon -- coordinator \
  --arena --next --candidate 'multiloop:kmeans=4:seed=42'
```

挙動:

1. `:00 / :15 / :30 / :45` の次の試合ウィンドウ（開始 3 分前 open）まで待機。
2. window 内だけで `GET /settings`（token 付き）を **10 秒間隔**で poll して参加登録。
   - `403 AccessTimeError` は「まだ登録できない」意味で、PR #165 の retry semantics
     （`coordinator_loop::retry_on_access_time`、`POST /agent` / `GET /` と**同一**の helper）で再試行。
   - それ以外（401 / 5xx / transport）は即エラー。予算（window から算出、cap 60 発）が
     終わったら `registration_failed` として記録して終了。
3. 登録後はいつもの coordinator フロー（`GET /setting` → `POST /agent` → per-day loop）で
   **1 試合だけ**プレイし、evidence を追記して**終了**（idle しません）。

### daySteps は地形加重ステップ予算（Issue #226）

公式サーバは `POST /` を **地形加重**で検証する:
`Σ(進入セルの step cost: 平地 2 / 山 3 / 道路 順調1・混雑2・渋滞4) + Σ待機 == daySteps[day]`。
element 数（move = 1 ステップ）基準ではない（localhost probe: `[2,-19]` = 加重 21 → 拒否、
`[2,-18]` = 加重 20 → 受理）。候補は engine の element 時計で plan を組むため、
提出直前に `coordinator_loop`（`CandidateBridge` / `EngineLoopAdapter` の両 path が
ここを通りうる唯一の choke point）が `engine::wire_plan::normalize_day_plan` で
**行動 prefix を保ったまま truncate + 残りを待機ランで pad** してから送る。
候補本体・offline scoring は変更しない。この正規化が乗っているので、arena 投入時も
移動 plan が受理される（#226 以前の「`wait` のみ受理 / 移動は全日 revision=-1」は
旧地形表 + 加重超過の複合原因）。

フラグ（`procon coordinator`）:

| flag | 意味 |
|---|---|
| `--arena` | arena profile を有効化。server 既定 `PROCON_ARENA_SERVER`（未設定なら `https://procon37arena.online`）、token 既定 `PROCON_ARENA_TOKEN`。明示 `--server` / `--token`（`PROCON_SERVER` / `PROCON_TOKEN` 含む）が常に優先。evidence 追記 + 1 試合で終了 |
| `--next` | 次の cadence window まで待ってから登録 → プレイ（`--arena` が必須） |
| `--candidate '<id>'` | Issue #178 registry の candidate id を **verbatim** でプレイ（`--solver` より優先）。id は evidence に記録 |
| `--arena-evidence <PATH>` | evidence JSONL の場所（既定 `target/arena/runs.jsonl`） |

`--arena` なしの `procon coordinator` / `procon tournament` は **これまでと一字一句同じ**挙動です
（arena は純粋に opt-in。`live_state_slot` / `secret_redaction` はデフォルト `None`）。

## token / アカウント規律

- token は **self-hosted Infisical の `PROCON_ARENA_TOKEN` から runtime 注入**します。
  repo / fixture / log / CI artifact に保存しません。
- **CLI 引数として token を打つ手順は標準ではありません**（`--token` は既存 override として
  残していますが、arena 手順では使いません。`--arena-token` のような flag は存在しません）。
- `arena::Secret` が token を包み、`Debug` / `Display` は常に `[REDACTED]`。
  サーバ応答が token を echo してきても、error 文字列・state snapshot・evidence は
  書込み前に redact されます（`crates/cli/tests/arena_token_hygiene.rs` が sentinel で監査）。
- **表示名・アカウントデータはリポジトリに固定しません。** 表示名は arena の Web UI 側で管理
  （リポジトリ側からは変更しません）。token / email / 表示名は evidence に格納する
  フィールド自体が存在しません。

## evidence（durable record）

- パス既定: `target/arena/runs.jsonl`（`target/` は gitignored。要約は Issue へ、生 artifact はリポジトリへ）
- 1 行 = 1 試合。`record_kind` は **`"arena-run"`**。

| field | 内容 |
|---|---|
| `candidate_id` | shortlist の candidate id（`--candidate` を verbatim、`--solver` 使用時は null） |
| `solver` | 投入した solver configuration（mode / solver / loop 利みは knob または null） |
| `source_commit_sha` | `PROCON_SOURCE_COMMIT_SHA`（`just arena-next` が git head を自動注入） |
| `arena_server` / `team_id` | 投入先とチーム |
| `window_starts_at_unix` / `registration_attempts` / `registered_at_unix` / `arena_match_id` | 対象 match window と登録の事実 |
| `status` | `completed` / `rejected` / `error` / `registration_failed` / `api_drift` |
| `days_completed` / `revisions` | プロトコルから取れる result summary |
| `result_ref` | arena が result URL / id を公開した場合に記録（現状 null — live smoke 時に確定） |
| `error` | redact 済みの失敗要因 |

**offline の `RunRecord`（`target/eval/runs.jsonl`、`record_kind` なしの score tuple）と
混同しないこと** — schema・ファイル・命名系統が全部別物です。arena の result を
offline のスコアに合算・再集計する経路は作りません。

## drift fail-closed（推測しない）

非公式 arena は未認証・未知パスに **HTTP 200 の SPA HTML shell** を返します
（`<title>ヘキサうどん 非公式練習場</title>`）。したがって status code だけは信用しません:

- `protocol::MatchClient` は JSON 必須エンドポイントで **content-type と body 先頭**を検証。
  HTML / 非 JSON を検出した瞬間に `HttpClientError::ApiDrift` で**即失敗**（リトライなし）。
- `ApiDrift` は evidence 上も `api_drift` ステータスになります。
- **API drift を検出したら fail closed。request shape を推測で変えない、リトライストームを起こさない。**

## 外部サービスへの配慮

- poll は **pre-match window の中だけ**・10 秒間隔。cadence（15 分）より細かい
  submission / polling は行いません。
- 上記以外の高頻度アクセス・大量 sweep・Web scraping 前提の自動化は行いません
  （Scope Out と同義）。

## 同一盤面の再現（import hook）

arena が提供する同一盤面再現 JSON を offline corpus に取り込み、
arena で負けた case を #178 の evaluator で再実行できます:

```bash
# 1. replay JSON → CaseSpec（失敗 shape は fail closed）
cargo run -q --bin procon -- arena import-case \
  --input arena-replay.json --output target/arena/cases/loss-001.json

# 2. offline で再評価（bundled fixture + import を同一 corpus に載せる）
cargo run -q -p bench --bin procon-eval -- \
  --cases all \
  --case-files target/arena/cases/loss-001.json \
  --candidates 'multiloop:kmeans=4:seed=42,loop:iterations=32:initial=greedy' \
  --baseline loop:iterations=32:initial=greedy \
  --output-dir target/eval
```

- 変換は薄い converter のみ: 既知のポインタ（root / `setting` / `matchSetting` /
  `replay/setting` / `match/setting`）から wire `MatchSetting` を探し、
  evaluator と同じ validator（thresholds + wire→engine 変換）を通らないと出力しません。
- 出力は fixture-shaped（`case_id = fixture:<name>`）。`crates/cli/tests/fixtures/arena/`
  の sanitized sample（アカウントデータなし）でテストしています。
- `just arena-import <in> <out>` も同内容を提供します。

## 検証について

- テスト / CI は **`procon37arena.online` に一切リクエストしません。**
  fake arena（axum / mockito）だけで registration 403→200・HTML drift・
  cadence 予算・フルマッチ・token hygiene を検証します。
- 本物の arena に対する live smoke（token 環境での 1 試合参加）は**別作業**です。
  そのときだけ production traffic を発生させ、`result_ref` の実値を確定させます。
