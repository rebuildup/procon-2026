# 本選 Windows ネイティブ運用（試合記録・通信ログつき）

Windows 11 の PowerShell だけで `procon.exe` を動かす手順です。WSL、Docker、Git Bash、Infisical、インターネット接続は使いません。Mac 用の手順は [`honsen-runbook.md`](honsen-runbook.md) にあります。

試合の記録と HTTP 通信ログは Rust 側（`procon.exe coordinator`）が書きます。起動スクリプト [`ops/honsen/honsen.ps1`](../../ops/honsen/honsen.ps1) は、記録先の指定と起動前検査、落ちたときの再起動を受け持つだけです。

> **Windows 実機では未検証です。** 下の「検証状況」を読んでから使ってください。

## 1. 準備（インターネットにつながる場所で一度だけ）

```powershell
# Rust (MSVC) と Visual Studio Build Tools の「C++ によるデスクトップ開発」が必要
git clone https://github.com/rebuildup/procon-2026.git
cd procon-2026

# どのコミットのバイナリかを記録に残すため、ビルド時に埋め込む
$env:PROCON_SOURCE_COMMIT_SHA = (git rev-parse HEAD)
cargo build --release --locked -p cli

New-Item -ItemType Directory -Force "$HOME\procon-honsen\bin" | Out-Null
Copy-Item target\release\procon.exe, ops\honsen\honsen.ps1 "$HOME\procon-honsen\bin\"
```

以降は `%USERPROFILE%\procon-honsen\bin` で作業します。競技中のビルドは不要です。

## 2. 競技開始前

```powershell
cd "$HOME\procon-honsen\bin"
powershell -ExecutionPolicy Bypass -File .\honsen.ps1 preflight -Server http://<IP>:<ポート>
```

トークンを聞かれるので入力します（画面には出ません）。`PREFLIGHT OK` が出れば準備完了です。

| 表示 | 意味と対処 |
|---|---|
| `OK 試合記録: ...\records` | 記録先に実際に書けた |
| `FAIL 試合記録が無効です` | 古い `procon.exe` を使っている。手順 1 でビルドし直す |
| `FAIL 試合記録の保存先に書けません` | 表示されたフォルダの権限と空き容量を確認する |
| `OK HTTP 通信ログ: ...\logs` | 通信ログの保存先に実際に書けた |
| `OK サーバー応答: 200` / `403 AccessTimeError` | 認証 OK（403 は試合がまだ設定されていないだけ） |
| `FAIL サーバー応答: 401 AuthError` | トークン違い。入れ直す |
| `FAIL サーバーに接続できません` | LAN ケーブル、IP、ポートを確認する |
| `FAIL 別の procon.exe が動いています` | タスクマネージャーで止める |

preflight がサーバーへ送るのは `GET /setting` 1 回だけです。

## 3. 競技開始時

車種の回答受付が始まる前に起動します。

```powershell
powershell -ExecutionPolicy Bypass -File .\honsen.ps1 start -Server http://<IP>:<ポート>
```

preflight をもう一度通してから `procon.exe coordinator` を起動します。プロセスが落ちた場合は 1 秒後に自動で起動し直し、試合の途中の日から続けます（開始から 40 分まで）。最終日の受理後に自動で終了し、記録の要約を表示します。

**試合中はコンソールの中をクリックしないでください。** 旧来のコンソールは範囲選択が始まると出力が止まり、`procon.exe` も止まります。スクリプトは起動時に簡易編集モードを切りますが、Windows 実機での確認はまだです。

スクリプトを使わずに直接起動する場合は次のとおりです。

```powershell
$env:PROCON_TOKEN = '<トークン>'
$env:PROCON_RESUME_RUNNING_MATCH = '1'
.\procon.exe coordinator --server http://<IP>:<ポート> --exit-after-match `
  --candidate 'fleet:budget=official:budget_ms=10000' --kinds-auto `
  --record-dir "$HOME\procon-honsen\records" `
  --http-log-dir "$HOME\procon-honsen\logs"
```

`--record-dir` / `--http-log-dir` の代わりに環境変数 `PROCON_RECORD_DIR` / `PROCON_HTTP_LOG_DIR` でも指定できます。指定がなければ記録は取られず、起動時に `match recording is OFF` / `HTTP log is OFF` の WARN が出ます。

起動後は、HTTP の 1 往復ごとにステータスと送受信の全文がコンソールに流れます。読み方は [`http-log.md`](http-log.md) にあります。

```text
[HTTP #16] 2026-10-10T02:27:56.619Z POST http://192.168.1.10:8080/ -> 200 OK (4 ms) attempt=1 day=0
  > Procon-Token: [MASKED]
  > Content-Type: application/json
  > body: [[4,4,5,-10,2,2,3,3],[4,4,3,1,2,2,3,4]]
  < content-type: application/json
  < body: {"revision":1}
```

`-> 200 OK` で `revision` が 0 以上なら受理です。`-> FAILED timeout` / `-> FAILED connect` が続くときは LAN ケーブルを確認します。`-> 401 Unauthorized` はトークン違いです。

## 4. 競技終了後

```powershell
powershell -ExecutionPolicy Bypass -File .\honsen.ps1 show          # 最新の記録を読み戻して要約
Compress-Archive "$HOME\procon-honsen\records", "$HOME\procon-honsen\logs" "$HOME\procon-records.zip"
```

`show` は `procon.exe record show <run ディレクトリ>` を呼びます。日ごとに、状態を観測できたか、何台分・何手の行動を提出したか、`POST /` の回数、サーバーの判定と revision が出ます。`unparsed_lines` と `missing_seq` が 0 なら、記録に欠けはありません。

## 5. 保存されるもの

`%USERPROFILE%\procon-honsen\records\<run-id>\` に、起動 1 回につき 1 ディレクトリができます。形式の詳細は [`match-record.md`](match-record.md) にあります。

| ファイル | 内容 |
|---|---|
| `manifest.json` | source commit、バイナリ SHA256、candidate / solver 設定、車種編成、開始時刻、OS / arch、結果 |
| `setting.json` | `GET /setting` の内容（マップ全セル、スポット、初期位置、日数、ステップ数、制限時間、燃料上限、混雑・渋滞の閾値） |
| `events.jsonl` | 日ごとの `GET /` の実観測、提出した行動列の全文、`POST /` の各試行と結果、所要時間 |
| `coordinator.log` | ログ（INFO 以上。`PROCON_RECORD_LOG_LEVEL=debug` で詳細化） |
| `state.json` | 既存の `--state-output` と同じ形式の最新スナップショット |

`%USERPROFILE%\procon-honsen\logs\<run-id>\http.jsonl` には、HTTP の 1 往復ごとの記録（ステータス、リクエストとレスポンスのヘッダーと本文の全文、失敗の種別、所要時間）が入ります。`run-id` は試合記録と共通です。

トークンはどのファイルにもコンソールにも出ません。

## 6. 困ったとき

| 症状 | 対応 |
|---|---|
| `match-record: writing ... failed` が出る | 記録の書込みに失敗している。**試合は続いています。止めない。** 試合後にディスクを確認する |
| `match record is incomplete` の ERROR | 記録の一部が欠けた。試合には影響しない |
| `http-log: writing ... failed` が出る | 通信ログのファイルに書けていない。コンソールへの表示は続く。**試合は続いています** |
| `honsen.ps1` が動かない | 手順 3 の直接起動コマンドを使う |
| 端末が再起動した | もう一度 `honsen.ps1 start`。新しい run ディレクトリに続きが書かれ、`manifest.json` の `continues_run_id` が前回を指す |

## 検証状況

| 項目 | 状況 |
|---|---|
| 記録の内容と読み戻し（16×16 / 24×24 / 32×32、拒否、再送、kill -9 後の再起動、書込み失敗） | macOS arm64 で検証済み（[`docs/evidence/record-394/`](../evidence/record-394/README.md)） |
| HTTP 通信ログ（200 / 4xx / 5xx / タイムアウト / 切断 / JSON でない応答、コンソールとファイルの一致） | macOS arm64 で検証済み（[`docs/evidence/http-log-396/`](../evidence/http-log-396/README.md)） |
| `honsen.ps1` の preflight / start / show | macOS 上の PowerShell 7 と公式簡易サーバー、モックサーバーで確認 |
| Windows 11 での `procon.exe` のビルドと実行 | **未検証** |
| Windows PowerShell 5.1 での `honsen.ps1` | **未検証** |
| 簡易編集モードの無効化 | **未検証** |
| 本選の実サーバーへの接続 | **未検証**（URL とトークンは当日配布） |
