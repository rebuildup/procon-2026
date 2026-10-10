# 試合記録の形式（`--record-dir`）

`procon coordinator --record-dir <DIR>`（または `PROCON_RECORD_DIR`）は、起動 1 回につき `<DIR>/<run-id>/` を作り、試合の再現に必要な情報を保存します。実装は [`crates/cli/src/match_record.rs`](../../crates/cli/src/match_record.rs)、schema version は `1` です。

```text
<DIR>/<run-id>/
  manifest.json
  setting.json
  events.jsonl
  coordinator.log
  state.json
```

`run-id` は `<UTC 開始時刻>-p<pid>`（例 `20261010T021226Z-p53464`）で、こちらが付ける名前です。公式 API は試合 ID を返さないので、`manifest.json` の `match_identity.official_match_id` は常に `null` です。

## 試合への影響

記録は観測であり、回答の経路には入りません。

- HTTP 通信を増やしません。記録するのは coordinator がもともと行う `GET /setting`・`POST /agent`・`GET /`・`POST /` の内容だけです。
- 試合処理のスレッドが行うのは、上限 8192 件のキューへの非ブロッキング送信だけです。JSON 化、マスク、書込み、`fsync` は記録用のスレッドが行います。
- `coordinator.log` の行は、キューが半分（4096 件）埋まった時点で先に捨てます。ログが大量に出ても、行動列や判定の記録を押し出すことはありません。
- キューが満杯なら、その記録を捨てて件数を数えます。書込みに失敗したら件数を数え、stderr に `match-record: writing ... failed` を出し、次の記録でファイルを開き直します。どちらの場合も試合は続きます。
- 欠けた件数は日ごとに `match record is incomplete` の ERROR で出ます。ファイル上では `seq` の欠番になります。
- 記録先を作れなかった場合は、起動時に ERROR を出して記録なしで試合を続けます。

1 日ぶんの記録呼び出しが試合処理のスレッドに与える負荷は、32×32・8 台・交通情報 1024 件の状態で平均 5 µs、最大 73 µs でした（release ビルド、macOS arm64、200 日ぶんの計測）。

## 強制終了したときに残るもの

イベントは 1 行ごとに書込みと `fsync` を済ませます。`kill -9` で失われるのは、その瞬間にキューに残っていた記録だけです（通常は 0 件）。

提出行動列（`plan`）は `POST /` の直前にキューへ入れますが、ディスクへの書込み完了は待ちません。遅いディスクが回答を遅らせないようにするためです。したがって、電源断の瞬間に書込み中だった 1 件は残らないことがあります。途中で切れた行は読み戻しで飛ばされ、`unparsed_lines` に数えられます。

## 再起動

プロセスを起動し直すと、新しい run ディレクトリができます。試合の途中から再開した場合（`PROCON_RESUME_RUNNING_MATCH=1`）は、同じ `<DIR>` の中から同じ試合の前回 run を探し、`manifest.json` の `continues_run_id` と `continues` イベントで指します。前回 run のファイルには触りません。

同じ試合かどうかは、`setting.json` が `startsAt` を除いて一致し、かつ `startsAt` が一致することで判定します。前回 run が試合開始前に終わっていて `startsAt` を知らない場合は、30 分以内に始まった run に限り `matched_by: "setting_only"` として結びます。

## manifest.json

| キー | 内容 |
|---|---|
| `schema_version` / `run_id` / `started_at` / `started_unix_ms` / `pid` | 記録の識別と開始時刻（UTC） |
| `source_commit_sha` / `source_commit_sha_from` | ソースのコミットと、その出どころ（`runtime-env` / `build-env` / `git-working-directory`） |
| `binary` | 実行ファイルのパス、SHA256、サイズ |
| `os` / `arch` / `package_version` | 実行環境 |
| `server` / `arena` | 接続先 |
| `solver` | candidate ID、`--solver`、loop の設定、`--kinds` / `--kinds-auto` |
| `agent_kinds` | 実際に申告した車種編成（wire の値、`P,S,…` 表記、決め方） |
| `match` | `starts_at`、日数、マップの幅と高さ、台数、チーム数 |
| `resumed_at_day` / `continues_run_id` | 途中参加した日と、前回 run |
| `result` | `completed` / `rejected` / `error`、受理日数、revision の列 |
| `recorder` | 採番したイベント数（`events_numbered`）、捨てた件数（`dropped`）、書込み失敗の件数（`write_errors`）。試合終了時とプロセス終了時に書く |

コマンドライン引数は保存しません（`--token` が含まれうるため）。

トークンのマスクは、JSON の文字列値に対して行います（`coordinator.log` は行の文字列に対して）。数値やキー名は書き換えないので、トークンがどんな文字列でも JSON が壊れることはありません。

## setting.json

`GET /setting` の応答を、公式の wire 形式（キー名、地形コード 0〜3、スポット、初期位置）のまま保存します。`procon arena import-case --input setting.json --output case.json` で、そのままオフライン評価器（`procon-eval --case-files`）のケースになります。

保存するのは `protocol::MatchSetting` に定義されたフィールドです。公式仕様にないフィールドをサーバーが返した場合、それは保存されません。

## events.jsonl

1 行 1 イベントの JSON です。共通のキーは次のとおりです。

| キー | 内容 |
|---|---|
| `v` | schema version |
| `seq` | この run での通し番号（欠番は記録の欠落） |
| `run_id` | run ディレクトリ名 |
| `ts` / `unix_ms` | 発生時刻（UTC、`coordinator.log` と同じ時計） |
| `mono_ms` | 記録開始からの経過（単調時計） |
| `type` | イベントの種類 |

| `type` | 内容 |
|---|---|
| `setting` | `GET /setting` が取れた。`starts_at` と、それまでに待った 403 の回数 |
| `kinds` | 申告する車種編成 |
| `agent_submit` | `POST /agent` の結果と所要時間 |
| `match_start` | サーバーが告知した開始時刻 |
| `resume` / `continues` | 途中参加した日と、前回 run |
| `day_state` | その日の `GET /` の応答（`observed`）。自チームの位置・燃料・種類、他チーム、交通状態、`endsAt` |
| `days_skipped` | 回答を試みないまま過ぎた日 |
| `plan` | その日に `POST /` へ渡した行動列の全文（`actions`）、ステップ数、計算時間 |
| `plan_failed` | 行動列を作れなかった理由 |
| `submit_attempt` | `POST /` 1 回ごとの結果（`accepted` / `rejected` / `no_verdict` / `window_closed` / `client_error` / `fatal`）、revision、所要時間、エラー文 |
| `day_result` | その日の判定（`accepted` / `rejected` / `unanswered`）、revision、試行回数、`GET /` から判定までの時間、締切までの余裕 |
| `request_failed` | 失敗して再試行した `GET /` |
| `match_end` | 試合の結末 |

`observed` と `actions` の中身は公式の wire 形式そのままです。

**提出と受理は別のイベントです。** `plan` は送ろうとした内容、`submit_attempt` は 1 回の送信とサーバーの返答、`day_result` はその日の結論です。再送があれば `submit_attempt` が複数行になります。

**観測と予測は混ぜていません。** `day_state.observed` はサーバーが返した値だけです。行動列から計算した到達位置はどこにも保存していません。計画どおりに動いたかは、`plan` と翌日の `day_state` を比べて確かめます。

## 読み戻し

```bash
procon record show <DIR>/<run-id>
```

`manifest.json`・`setting.json`・`events.jsonl` を読み、日ごとの要約を JSON で出します。`missing_seq` は欠けたイベントの数で、ファイルの途中の欠番に加え、`manifest.json` の `recorder.events_numbered` と比べて末尾の欠けも数えます。プロセスが強制終了されて `recorder` が書かれなかった run では、最後に残った行より後の欠けは数えられません。Rust からは `cli::match_record::load_record` が同じ内容を型つきで返します。

## state.json / coordinator.log

`state.json` は `--state-output` と同じ schema（`schemaVersion: 1`）で、記録用スレッドが run ディレクトリへ書きます。`--state-output` を別に指定した場合、そのファイルの挙動は変わりません。

`coordinator.log` は tracing の出力で、既定は INFO 以上です。`PROCON_RECORD_LOG_LEVEL=debug` で詳細化できます（`RUST_LOG` と同じ書式）。最初の行に `run_id` が出るので、`events.jsonl` と時刻で突き合わせられます。
