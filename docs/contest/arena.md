# 非公式 arena 実行ガイド（Issue #179）

> 非公式練習場 `https://procon37arena.online` を **selected candidate の外部検証先**として
> 使うための実行・安全・記録のガイドです。
> 選抜方針と投入候補は Issue #179 の **shortlist comment**（supervisor 選抜 4 本）を参照。
> offline 側の評価経路は [docs/evaluation.md](../evaluation.md)（Issue #178）が唯一の canonical です。
>
> **arena 側の仕様出典**: 公式ガイド
> [`https://procon37arena.online/arena/guide`](https://procon37arena.online/arena/guide)
> （client-rendered。受付＝`GET /setting` + lobby 確認メッセージ、token の二段渡し、
> 参加受付の流れはこのガイドの記述が準拠。Issue #222 の supervisor 確認 2026-09-27）。

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

## 仕様の出典（公式ガイド）

操作仕様の正本は arena の公式ガイド
**<https://procon37arena.online/arena/guide>**（SPA のため本文はクライアント側 JS に格納）。
本実装が依拠する記述:

- **参加表明**: 受付（lobby）中にトークン付きで公式プロトコルの任意 endpoint
  （例 `GET /setting`）を叩くとその試合への参加表明。応答は本番同様の
  `403 AccessTimeError` で、メッセージに `lobby: you are registered …` が入る。
  **先着順**で参加チームが決まる（arena だけの仕組み）。
- **token の渡し方**: 本番と同じ `?token=` または `Procon-Token` ヘッダ（双対応）。
- **再接続ループ**: 試合終了でクライアントは切断し、次の受付中に再接続すれば次に参加。
  クライアント側をループにしておくと放置で回る（= 本実装の `--next` 設計）。
- **マップ/パラメータ**: マップは 16/24/32×32。日数・ステップ数・燃料積載量上限・
  混雑/渋滞基準値は本選まで非公開で、**参考マップ相当の値と公式範囲内の乱数が半々**。
  candidate は未知の日数・形状に耐えること（live では 10 日マッチを観測:
  `daySeconds` 60×10、`daySteps` 最大146、24×24 map）。

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
2. window 内だけで `GET /setting`（token 付き・**単数形**）を **10 秒間隔**で poll して参加登録。
   - **path の注記（Issue #222）**: #179 の案内要約は「`/settings` 等」と複数形で書いていたが、
     複数形は arena SPA の fallback ルート（`text/html` を返すだけ）で、実 API は公式 OpenAPI
     `ProconApi_getMatchSetting` の**単数 `/setting`**。live smoke（2026-09-27）で複数形に向いた
     registration が HTML を受け、drift fail-closed が正しく `api_drift` として停止して根本原因を
     検出した（token 漏れゼロ・1 request）。request shape は推測で変えず path のみ修正した。
   - **成功条件は 2 種類**（live recon 2026-09-27 で確認）:
     1. `403 AccessTimeError` の message が
        `lobby: you are registered for match <id> departing at <unix>` の形なら
        **登録済みの合図** — poll を止めて `registered_at` / `arena_match_id` /
        departure unix（evidence では `window_starts_at_unix`）を記録し、match 開始を待つ。
     2. `200` JSON なら従来どおり登録成功。
   - 上記以外の `403 AccessTimeError`（まだ lobby に入れない）は「まだ登録できない」意味で、
     PR #165 の retry semantics
     （`coordinator_loop::retry_on_access_time`、`POST /agent` / `GET /` と**同一**の helper）で再試行。
   - それ以外（401 / 5xx / transport）は即エラー。予算（window から算出、cap 60 発）が
     終わったら `registration_failed` として記録して終了。
   - **参加表明は先着順**（ガイド記述）。受付中の 1 回の `GET /setting` がそのまま slot を
     消費するので、成功後は poll を**必ず止める** — 成功後に再登録を試みる retry はしない
     （実装: `arena::wait_for_registration` の lobby 成功 branch 即 return）。
3. 登録後はいつもの coordinator フロー（`GET /setting` → `POST /agent` → per-day loop）で
   **1 試合だけ**プレイし、evidence を追記して**終了**（idle しません）。
   - **再接続ループが公式の想定**（ガイド: 試合終了でクライアントは切断し、次の参加受付中に
     再度接続すればそのまま次の試合へ）。本ドキュメントの `--next` 1 試合完走で終了する設計と
     一致 — `just arena-next` を window ごとにループさせれば放置で回せる。
   - arena の match は **10-day**（live 観測 2026-09-27 20:30:01 JST: `daySeconds` 60×10、
     `daySteps` 最大 146、24×24 map）。offline corpus の small 盤とは**別物**として扱う。
   - **パラメータは半々**: マップは 16 / 24 / 32×32、日数・ステップ数・燃料積載量上限・
     混雑/渋滞基準値は本選まで非公開で、「参考マップ相当の値と公式範囲内の乱数を半々」
     （ガイド記述）。つまり **day 数は実行ごとに不定**（live では 10-day を観測）—
     candidate は未知の day 数に耐えること（`daySteps.len()` に依存した打ち切りをしない）。

### daySteps は地形加重ステップ予算（Issue #226）

公式サーバは `POST /` を **地形加重**で検証する:
`Σ(**出発セル**の step cost: 平地 2 / 山 3 / 道路 順調1・混雑2・渋滞4) + Σ待機 == daySteps[day]`。
（コストが課されるのは **移動元のセル** — 平地→道路の `[2,-19]`=加重21 で拒否 / `[2,-18]`=20 で受理、
平地→山地の `[2,-17]`=加重19 で拒否（進入セル基準なら20で受理されるはず）の3本 probe で確定。）
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
| `source_commit_sha` | `PROCON_SOURCE_COMMIT_SHA`（`just arena-next` が git head を自動注入）。未設定時は `git rev-parse HEAD` にフォールバックするので、`cargo run` 直接起動でも `unknown` にならない（Issue #222） |
| `arena_server` | 投入先 arena |
| `team_id` | coordinator のローカル team id。既定は **`mvp-player`**（`PROCON_TEAM_ID` / `CoordinatorLoopConfig::default`）。arena は登録応答に team id を返さないため、登録前でもこの既定値が入る — arena 側のアカウント ID ではない |
| `window_starts_at_unix` / `registration_attempts` / `registered_at_unix` / `arena_match_id` | 対象 match window と登録の事実 |
| `status` | `completed` / `rejected` / `error` / `registration_failed` / `api_drift` |
| `days_completed` / `revisions` | プロトコルから取れる result summary |
| `day_steps` / `day_seconds` | `GET /setting` の per-day 予算・回答時間枠（`daySteps` / `daySeconds`）。**1回だけ**記録し、`days[]` の各エントリはこの予算に対して読む（Issue #236） |
| `days` | **日ごとの evidence（Issue #236）**。1試合につき played day 数ぶんの要素（`days_completed` と同じ上限）。各要素は `day`（0-based index）+ `agents`（day 開始時点の自エージェント `kind / pos / fuel` — wire `GET /` の day state そのもの）+ `movement`（その日に**提出した**移動の要約: `moves` 移動数 / `waits` 待機ステップ数合計 / `runs` 待機ラン数 / `dirs` 方向ヒストグラム・公式 wire code `0..=5` 順）。**生の answer body は入れない** — 要約だけで、9日・8エージェント試合でも JSONL 1行のまま size guard テストが保証（< 16KB、1日あたり ~0.7KB、日数に対して線形） |
| `result_ref` | arena が result URL / id を公開した場合に記録（現状 null — live smoke 時に確定） |
| `error` | redact 済みの失敗要因 |

### なぜ日次要約か（#233 方針5 / Issue #236）

従来の evidence は registration / status / revisions / days だけで、**「何を動いたか・
燃料がどう減ったか」が無い**。敗因解析のたびに replay JSON を手動取得する羽目になり、
しかも `arena import-case` は MatchSetting しか取り込めない（日次 state・提出 action・
相手の交通状態は落ちない — この制約自体は本 Issue の非スコープ）ため、live で既に
見ていた情報が evidence に残らないだけの状態だった。

`days[]` があれば **replay JSON なしで** 次の敗戦を解析できる:

- **燃料 timeline**: 連続日次の `fuel` 差分で枯渇する日・エージェントを特定（#233 の
  match 489 postmortem で欠けていた軸）。
- **提出内容**: `moves` / `waits` / `runs` / `dirs` で「動かなかった」「同じ方向に
  往復していた」を日単位で特定。
- **判定前提**: `day_steps` / `day_seconds` でその日の予算・時間超過の有無を読む。

データ源は coordinator が **既に見ている** `GET /` の day state と提出 payload のみ
（`run_coordinator_loop` の revisions と同じ場所に並べて保持）。protocol client・
新規 request・新規 endpoint は一切増やしていない。`kind / pos / fuel` は game state
であり token / email / 表示名フィールドは依然として schema に存在しない（hygiene
テスト維持）。

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
- 日次 evidence（Issue #236）も fake arena で検証:
  `arena_fake_server` が played day ぶんの `days[]`（kind / pos / fuel + 提出移動要約、
  wait-only 提出なので `moves + waits == daySteps`）と top-level `day_steps` /
  `day_seconds` の充填を assert、`arena::tests::nine_day_record_stays_one_modest_jsonl_line`
  が size guard（9日・8エージェント合成レコード < 16KB、1日エントリ < 1KB、
  日数に対して線形、field count 固定）を assert します。
- 本物の arena に対する live smoke（token 環境での 1 試合参加）は**別作業**です。
  そのときだけ production traffic を発生させ、`result_ref` の実値を確定させます。
