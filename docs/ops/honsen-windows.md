# Windows 11 本選オフライン実戦手順（v0.5.0）

**対象:** 第37回高専プロコン競技部門。本選の Windows 11 ネイティブ + PowerShell 5.1 用。会場では WSL・Docker・Git Bash・Infisical・インターネット接続・ビルドを不要にする。**事前にこのファイルを OFFLINE-RUNBOOK.md として procon.exe と同じフォルダにコピーする。**

- 本番用の URL とトークンは運営から受け取る。練習 Arena の URL・トークンを流用しない。
- 以下の 192.168.1.10:8080 は仮の例。必ず当日配布の URL に置き換える。
- トークンをソース、テキストファイル、コマンドの引数、履歴に書かない。
- **HTTP 200 は通信成功であり、回答受理の保証ではない。revision と日別判定も見る。**

## 1. 会場で使うコマンド（最重要）

事前にコピーした bin ディレクトリで Windows PowerShell を開く。通常の PowerShell 実行ポリシーを回避できるよう、以下のように powershell.exe -File で実行する。

```powershell
cd "$HOME\procon-honsen\bin"
$Server = 'http://192.168.1.10:8080'  # 当日の本選URLに変更
powershell -NoProfile -ExecutionPolicy Bypass -File .\honsen.ps1 preflight -Server $Server
powershell -NoProfile -ExecutionPolicy Bypass -File .\honsen.ps1 start -Server $Server
```

- preflight は **GET /setting の1回のみ**で、車種・行動の POST は行わない。PREFLIGHT OK、試合記録 OK、HTTP 通信ログ OK を確認。
- start は自動的にもう一度 preflight を行ってから参加する。車種受付が始まる前に起動し、試合終了まで閉じない。
- preflight と start は別プロセスのため、環境変数を設定していなければ**それぞれでトークンを入力**する。入力は画面に表示されない。
- 別の start や procon.exe を同時に起動しない。コンソール内をクリックして文字列選択すると処理が止まる場合がある。

## 2. 会場へ行く前の準備（ネット接続がある場所）

本番で使用する Windows 11 実機上で準備。Rust の MSVC toolchain と Visual Studio Build Tools「C++ によるデスクトップ開発」がビルド時のみ必要。リリースPR #400が main に**マージ済み**であることを確認してから以下を実行する。

```powershell
git clone https://github.com/rebuildup/procon-2026.git
cd procon-2026
git fetch origin --tags
git switch main
git pull --ff-only origin main
git log -1 --format='%H %s'

$env:PROCON_SOURCE_COMMIT_SHA = (git rev-parse HEAD)
cargo build --release --locked -p cli
if ($LASTEXITCODE -ne 0) { throw 'release build failed' }

$Dest = Join-Path $HOME 'procon-honsen\bin'
New-Item -ItemType Directory -Force $Dest | Out-Null
Copy-Item .\target\release\procon.exe $Dest -Force
Copy-Item .\ops\honsen\honsen.ps1 $Dest -Force
Copy-Item .\docs\ops\honsen-windows.md (Join-Path $Dest 'OFFLINE-RUNBOOK.md') -Force
Copy-Item .\docs\ops\match-record.md $Dest -Force
Copy-Item .\docs\ops\http-log.md $Dest -Force

& (Join-Path $Dest 'procon.exe') version
Get-FileHash (Join-Path $Dest 'procon.exe') -Algorithm SHA256
Get-FileHash (Join-Path $Dest 'honsen.ps1') -Algorithm SHA256
Compress-Archive -Path (Join-Path $Dest '*') -DestinationPath (Join-Path $HOME 'procon-honsen-offline.zip') -Force
```

**バージョン 0.5.0 と SHA256 を確認。** git clone 済みなら clone は省略してよい。ビルド時の commit SHA は試合記録の manifest に残る。競技中は build・pull を実行しない。完成した bin と zip を PC・USB など二重に保持する。ビルドできない場合は事前作成済みの**同じ Windows/MSVC 用**バイナリと本スクリプトを持ち込む（出所とSHA256を確認）。

## 3. 本番直前チェック

1. 電源・ACアダプター・LANケーブル・時計・空き容量（1GB以上）を確認。時刻合わせは外部接続が使えるうちに済ませる。
2. bin に procon.exe と honsen.ps1 があるか確認。Windows PowerShell を開き、手順1の $Server を当日の本選URLで設定。
3. preflight を実行し、チームの本選トークンを安全入力。PREFLIGHT OK なら開始前の準備完了。
4. 車種回答受付の前に start。開始後はコンソールの表示を監視して放置し、不要な操作をしない。

| preflightの表示 | 意味・対応 |
|---|---|
| 200 | 設定取得成功 |
| 403 | 試合未設定時の想定応答。ただしスクリプトは本文を検証せず403をOK扱いするため、想定外の403は運営へ確認 |
| 401 | トークンが誤っている。入れ直す |
| FAIL 接続不可 | URL・LAN・IP・ポートを確認 |
| FAIL 記録が無効 / HTTPログが無効 | 古い procon.exe の可能性。事前にversion・SHA256を確認 |
| FAIL 記録先に書けない / 容量不足 | 保存先の権限・空き容量を確認 |
| FAIL 別の procon.exe | 二重起動を解消。現在試合中の既存プロセスは安易に止めない |

## 4. マージ後のコマンド・戦略の違い

**v0.5.0 による破壊的なCLI変更はない。** 従来の coordinator コマンドは引き続き使える。追加点は起動スクリプトとログ用オプション。

| 項目 | 動作 |
|---|---|
| 新しい推奨コマンド | honsen.ps1 preflight / start / show |
| start の既定候補 | fleet:budget=official:budget_ms=10000 |
| 車種 | --kinds-auto |
| 終了・復帰 | --exit-after-match、PROCON_RESUME_RUNNING_MATCH=1 |
| 記録保存 | --record-dir または PROCON_RECORD_DIR |
| HTTP全文 | --http-log-dir または PROCON_HTTP_LOG_DIR |
| 省略時 | 記録・HTTP全文はOFF（警告を表示） |

**リリースマージだけでは候補は変わらない。** 予行演習の上位候補が正式採用された場合のみ、事前に確認した Candidate ID を -Candidate で明示する。次は**書式の例**で、採用を意味しない。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\honsen.ps1 start -Server $Server -Candidate 'fleet:budget=official:iters=60000:chains=4:seed=3'
```

競技中の場当たり的な候補変更は避ける。車種登録30秒、日別回答60秒という制約を意識する。

## 5. エラー・異常終了・直接起動

- start は coordinator が異常終了すると **1秒後に自動再起動**し、前の試合の途中への復帰を試みる。開始から最大40分。完了後に拒否された日がある場合は再起動しない。
- PC・PowerShell 自体が落ちた場合は、同じ bin で URL を設定し直して start を再実行する。新しい run-id で続きが保存される。
- 先に動いている start を残したまま別の start や直接起動をしない。
- 記録書込みエラーだけなら **試合を止めず**、終了後にログを回収する。
- スクリプトが使えず停止している場合の**緊急直接起動**（自動再起動・二重起動検査がなくなる）：

```powershell
cd "$HOME\procon-honsen\bin"
$Server = 'http://192.168.1.10:8080'  # 本選URLに変更
$secure = Read-Host -AsSecureString '本選トークン'
$ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
try { $env:PROCON_TOKEN = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr) }
finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr) }
$env:PROCON_RESUME_RUNNING_MATCH = '1'
.\procon.exe coordinator --server $Server --exit-after-match --candidate 'fleet:budget=official:budget_ms=10000' --kinds-auto --record-dir "$HOME\procon-honsen\records" --http-log-dir "$HOME\procon-honsen\logs"
Remove-Item Env:PROCON_TOKEN -ErrorAction SilentlyContinue
```

- .env は自動読込みされない。本番トークンは入力プロンプトを使うか、必要なときのみ環境変数で渡す。--token 引数やテキストファイルに保存しない。

## 6. 試合終了後（結果と全通信を回収）

```powershell
cd "$HOME\procon-honsen\bin"
powershell -NoProfile -ExecutionPolicy Bypass -File .\honsen.ps1 show

$Run = Get-ChildItem "$HOME\procon-honsen\records" -Directory | Sort-Object Name | Select-Object -Last 1
.\procon.exe record show $Run.FullName
Compress-Archive -Path "$HOME\procon-honsen\records","$HOME\procon-honsen\logs" -DestinationPath "$HOME\procon-honsen-result.zip" -Force
```

| ファイル | 内容 |
|---|---|
| records/<run-id>/manifest.json | source commit、バイナリSHA256、候補・車種・試合結果 |
| records/<run-id>/setting.json | マップ・全試合設定 |
| records/<run-id>/events.jsonl | 各日の状態、全行動列、提出試行・受理判定 |
| records/<run-id>/coordinator.log / state.json | 動作ログ・最新状態 |
| logs/<run-id>/http.jsonl | HTTPごとの200/4xx/5xx、全ヘッダー・本文・所要時間・失敗種別 |

**200 OKだけでなく、revision と day_result を確認する。** revision が -1 のときは拒否。show の unparsed_lines=0 / missing_seq=0 は記録の欠けが見つからなかった目安。ただし強制終了直前の未書込み分は検出できない場合がある。

トークンはマスクされるが、ゲームの状態や送受信データは記録される。zipの共有範囲を限定する。

## 7. 検証範囲と連絡先

リリースPR #400 では GitHub Actions の Windows PowerShell 5.1 + モックサーバーを用いて Windowsネイティブビルド・試合完走・記録/HTTPログ読み戻しを確認。**実際のWindows 11実機と本選サーバーでは未検証**のため、会場でのpreflightを省略しない。

参照： https://github.com/rebuildup/procon-2026/pull/400 、同梱の match-record.md / http-log.md 。この文書はネット接続なしで使えるよう自己完結させてある。
