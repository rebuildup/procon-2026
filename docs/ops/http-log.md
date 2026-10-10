# HTTP 通信ログ（`--http-log-dir`）

`procon coordinator --http-log-dir <DIR>`（または `PROCON_HTTP_LOG_DIR`）は、coordinator が行う HTTP リクエストを 1 往復ずつ、次の 2 か所へ同じ内容で出します。実装は [`crates/cli/src/http_log.rs`](../../crates/cli/src/http_log.rs) と、`crates/protocol` の `HttpObserver` です。

- **コンソール（stderr）**: 人が読む形
- **`<DIR>/<run-id>/http.jsonl`**: 1 行 1 往復の JSON

`--record-dir` と併用すると、同じ `run-id` を使います（`records/<run-id>/` と `logs/<run-id>/` が対応する）。試合記録の `manifest.json` には `http_log` としてファイルの場所が入ります。

## コンソールの表示

```text
[HTTP #20] 2026-10-10T02:28:22.894Z POST http://127.0.0.1:60987/ -> 503 Service Unavailable (1 ms) attempt=1 day=2
  > Procon-Token: [MASKED]
  > Content-Type: application/json
  > body: [[2,2,2,3,3,-34,4,4],[2,5,3,4,4,3],…（全文）]
  < content-length: 0
[HTTP #22] 2026-10-10T02:28:23.303Z POST http://127.0.0.1:60987/ -> 200 OK (2 ms) attempt=1 day=2
  > Procon-Token: [MASKED]
  > Content-Type: application/json
  > body: [[2,2,2,3,3,-34,4,4],[2,5,3,4,4,3],…（全文）]
  < content-type: application/json
  < content-length: 15
  < body: {"revision":3}
[HTTP #20] 2026-10-10T02:28:20.736Z GET http://127.0.0.1:60985/ -> FAILED timeout (5001 ms) attempt=1 day=1
  > Procon-Token: [MASKED]
  ! error: error sending request for url (http://127.0.0.1:60985/): operation timed out
[HTTP #19 DECODE ERROR] 2026-10-10T02:28:20.229Z GET http://127.0.0.1:60997/ day=1 : API drift on /: expected JSON, got a non-JSON body starting with "<html>502 Bad Gateway</html>…"
```

（上の `…（全文）` はこの文書での省略で、実際の出力は本文を切り詰めません。）

1 行目に、リクエスト番号、送信時刻（UTC）、メソッド、URL、結果、処理時間、試行番号、日が並びます。

| 結果の表示 | 意味 |
|---|---|
| `200 OK` / `400 Bad Request` / `401 Unauthorized` / `403 Forbidden` / `500 Internal Server Error` / `503 Service Unavailable` など | サーバーが返したステータス |
| `FAILED timeout` | 制限時間（5 秒）内に応答が完了しなかった |
| `FAILED connect` | 接続できなかった（ケーブル、IP、ポート、サーバー停止） |
| `FAILED request` | 送信中または応答の受信前に接続が切れた |
| `FAILED body` | ステータスは届いたが本文の途中で切れた（`after status …` が付く） |
| `DECODE ERROR` | 応答は届いたが、プロトコルが約束する JSON として読めなかった。直前の同じ番号の往復に本文がある |

`>` は送信、`<` は受信、`!` はエラー文です。`attempt` はクライアント内部の再試行の番号で、`GET` は一時的な失敗（5xx、タイムアウト、接続失敗）で最大 4 回やり直します。`POST` はクライアント内部ではやり直さず、coordinator が同じ日の回答窓の中で送り直します。どちらも 1 回の送信が 1 つの番号になります。

`day` は、直近に成功した `GET /` が返した日です。最初の `GET /` が成功するまでは `-` です。公式 API は試合 ID を返さないので、試合の識別には `run-id` を使います。

## 記録するのは実際に送受信した内容だけ

- 応答がなかった往復は `FAILED` と、HTTP ライブラリのエラー文だけを書きます。ステータスや本文を推定して埋めることはありません。
- 本文は省略も切り詰めもしません。
- リクエストヘッダーは、クライアントが設定した `Procon-Token`（マスク済み）と `Content-Type` を出します。HTTP ライブラリがその下で付ける `host`・`accept`・`content-length` は記録していません。
- レスポンスヘッダーは受け取ったものをすべて出します。UTF-8 として読めないバイトは `\xNN` の形で出します。
- **リダイレクトは 1 往復として記録されます。** HTTP ライブラリは 301 / 302 を自動でたどるので、その場合に残るのは最初の URL と最終的な応答だけで、途中の応答は残りません。公式サーバーはリダイレクトを返さないため、通信の挙動を変えないことを優先してこのままにしています。
- 一時的な失敗（5xx・408・425・429）で、このあとクライアントが再試行する応答は、本文を 200 ms だけ待ちます。届かなければ `FAILED body after status 503 …` として記録し、待たずに再試行します（ログなしのときはステータスを見た時点で本文を読まずに再試行するため）。

## 秘密情報

- `Procon-Token` の値は HTTP クライアントの中で `[MASKED]` に置き換えてから渡されます。
- 名前に `token`・`authorization`・`cookie`・`api-key`・`secret`・`password` を含むヘッダーは、送信・受信とも値を `[MASKED]` にします。
- トークンの文字列そのものが URL・本文・ヘッダー値・エラー文に現れた場合は `[REDACTED]` に置き換えます（サーバーが 401 の本文でトークンを返してきた場合など）。

## 試合への影響

- `--http-log-dir` を付けないとき、通信の経路は従来と同じです。フックを通りません。
- 付けたとき、試合処理のスレッドが行うのは上限 4096 件のキューへの非ブロッキング送信だけです。整形、マスク、ファイルへの書込みと `fsync` はログ用のスレッドが行い、コンソールへの出力はさらに別のスレッドが行います。ファイルを先に書くので、コンソールが出力を受け付けなくなっても（Windows の旧来のコンソールで範囲選択をした場合など）、リクエストもファイルへの保存も止まりません。ディスクが遅くても、リクエストとコンソール表示は止まりません。
- キューが満杯ならその 1 件を捨てて数え、終了時に `http-log: incomplete (...)` を stderr に出します。ファイルに書けないときは、コンソールへの表示を続けたうえで `http-log: writing ... failed` を出します。
- 応答の本文は、フックがあるときはクライアントが先に全部読んでからデコードします。フックがないときはデコード時に読みます。本文が途中で切れた場合の扱いは、フックの有無で同じになるようにしてあります（2xx の JSON 応答は再試行せず通信エラーとして返す、`POST /agent` の 2xx は成功のまま、2xx 以外はステータスどおりに再試行する）。この 3 通りと、本文が届かない 503 で再試行が遅れないことは、テストで両方の経路を比べています。

標準出力には何も足しません。通信ログのコンソール表示は stderr だけに出ます（従来の tracing ログは stdout のまま）。

## http.jsonl

| キー | 内容 |
|---|---|
| `type` | `exchange`（1 往復）または `decode_error` |
| `run_id` | 実行の識別子 |
| `n` | リクエスト番号（1 から）。`decode_error` は対象の往復と同じ番号 |
| `ts` / `unix_ms` | 送信時刻（UTC） |
| `day` | 直近の `GET /` が返した日。なければ `null` |
| `method` / `url` / `attempt` / `elapsed_ms` | リクエストと所要時間 |
| `result` | `response` または `failed` |
| `status` / `status_text` | HTTP ステータス（届いた場合） |
| `failure` / `error` | 失敗の種別（`timeout` / `connect` / `request` / `body` / `decode`）とエラー文 |
| `request_headers` / `request_body` | 送信した内容（本文は文字列で全文） |
| `response_headers` / `response_body` | 受信した内容（本文は文字列で全文） |
| `request_body_hex` / `response_body_hex` | 本文が UTF-8 として読めない場合だけ付く、バイト列そのままの 16 進表記。このとき `*_body` は読めないバイトを `�` に置き換えた表示用の文字列 |

コンソールの表示は、この各行を整形したものです。`cli::http_log::render` が同じ関数で、テストは「コンソール出力 = 全行の `render` の連結」を確認しています。

```powershell
# 例: 200 以外の往復だけを抜き出す（PowerShell）
Get-Content http.jsonl | ConvertFrom-Json | Where-Object { $_.status -ne 200 } |
  Select-Object n, ts, method, url, status, failure, error
```

## 検証

検証の記録は [`docs/evidence/http-log-396/README.md`](../evidence/http-log-396/README.md) にあります。Windows 実機では未検証です。
