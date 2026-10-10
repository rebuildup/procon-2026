# Windows 11 ネイティブ RC 検証 — `0a1fa5d`（release-0-5-1 @ `8b7324b` + #443 + #449 + 本修正）

実行日: 2026-10-11 JST / 検証ブランチ: `443-verify-rc`（`origin/release-0-5-1` @ `8b7324b` をマージ済み）
ソース SHA: `0a1fa5d1bc6cbdce4b477224aebcb81ff70f4e72` / exe SHA256: `E53E2F91CC2466A009CB495094B6D8EF15B16940B0A47202FB1B1ED3837D0A0D`
（exe は HEAD SHA を埋め込むためコミットごとに変わる。`windows-rc-21dd16c` の証跡とは別 SHA・別 exe）

環境: Windows 11 10.0.29683 / PowerShell 5.1.29683.1000 と pwsh 7.6.6 / rustc 1.99.0 (x86_64-pc-windows-msvc) / Python 3.14.8。
WSL・Docker 不使用。**本番サーバーには一切接続していない**（模擬サーバーは 127.0.0.1 のみ）。

## 判定

| 項目 | 判定 | 証跡 |
|---|---|---|
| Native MSVC release build（`--locked -p cli`） | PASS | `verify-rc-build-release.log` |
| exe SHA / HEAD SHA 照合 | PASS | `verify-rc-sha-check.log`, `exe-sha256.txt` |
| tournament_basic 6/6 | PASS | `verify-rc-tournament-basics.log` |
| ZIP 生成→別ディレクトリ展開→preflight/start/show→3 日受理→manifest SHA（**PS 5.1**） | PASS | `bundle-ps51-direct.log`, `verify-rc-bundle-e2e.log` |
| 同上を **PS 7**、空白入りパス、`.env` が古い環境変数を上書き、`records/` `logs/` が bundle 直下 | PASS | `ps7-extracted-zip-start.log`, `ps7-extracted-zip-mock-summary.json`（accepted_days=[0,1,2], bad_token=0） |
| e2e-mock（3 日・記録一式・HTTP ログ・record show 欠損 0・manifest SHA・ロック解除） | PASS | `verify-rc-e2e-mock.log` |
| 二重起動拒否 / kill 後のロック残骸 / ロック引継ぎ復帰 / 復帰試合 2 日受理 | PASS | `verify-rc-double-start.log` |
| 59s candidate（32×32・7 体・60s/日、既定 candidate、pwsh 7） | PASS（1 日） | `budget59-honsen.log`, `budget59-http-timeline.txt` |
| e2e-official（公式簡易サーバー） | BLOCKED/SKIP | バイナリが Windows 実機に無い |
| 本番サーバー接続・公式受理 | BLOCKED | 本番サーバー使用不可（既知の環境制約） |
| telemetry / monitor（#436） | SKIP | release 未統合（別 RC） |

59s candidate: 既定は `fleet:budget=official:budget_ms=59000:adapt=off` + `--kinds-auto`（honsen.ps1 の既定値が反映されている）。
1 日の実測: 日開始 17:13:06.2 → 提出 17:14:00.7（約 54.5 s、余裕約 5.5 s）、200 受理、timeout・再提出なし。
**1 日・1 盤面・mock の実測**であり、全日・複数盤面の締切余裕は #453 の記録に依存する。

## 発見した不具合と修正（commit `0a1fa5d`）

1. **`double-start` FAIL → 修正**: PowerShell 7 の module パス（`...\PowerShell\Modules`）を継承した環境で
   `powershell.exe`（5.1）を起動すると `Get-FileHash` が見つからず `honsen.ps1` が起動直後に落ちる。
   当環境のプロセス環境変数で再現（レジストリの `PSModulePath` は正常）。pwsh や Claude Code など PS7 系の親から 5.1 を起動した場合に発生。
   `honsen.ps1` / `verify-offline-bundle.ps1` の冒頭で、5.1（Desktop）のときだけ PS7 のパスを `PSModulePath` から外す。
   → `verify-rc-first-run-double-start-FAIL.log`（修正前）と `verify-rc-double-start.log`（修正後）。
2. **`bundle-e2e` FAIL → 修正**: `Start-Process -PassThru` + 出力リダイレクトでは、`WaitForExit(ms)` 後に `ExitCode` が `$null` になる
   （5.1 の既知の癖）。試合自体は 3 日完走していたのに「mock server rejected a day: exit 」と誤判定。`$Server.Handle` を先に参照して修正。
   → `verify-rc-first-run-bundle-e2e-FAIL.log`（修正前）。

どちらも検証スクリプト／起動スクリプト側の問題で、探索設定・通信プロトコル・締切マージンは変更していない。

## 未実施・残リスク

- 公式簡易サーバー／本番での受理は未検証（BLOCKED）。mock で再現できない公式固有挙動は検証済みと主張しない。
- 59s の全日・全盤面の時間予算は今回 1 日分のみ。
- 目視は未実施: ZIP 展開直後に別ユーザー環境で開く手順（実環境の PSModulePath が汚れていなければ修正 1 は不要だが害もない）。
- monitor（#429/#436）は未統合のため未検証。
