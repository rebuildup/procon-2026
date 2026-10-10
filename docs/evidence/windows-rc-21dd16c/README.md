# Windows 11 ネイティブ RC 検証証跡 — release-0-5-1 @ `21dd16c`

実行日: 2026-10-11（JST）
担当: Windows 実機検証エージェント（OpenCode / Supervisor + worker 分離）
対象 Release PR: #438 / 対象ブランチ: `release-0-5-1`

このディレクトリは、Windows 11 実機で**実際に実行した**検証の再現用ログのみを保持する。
実行していない項目を PASS と書かない。SKIP / 未実施は表にその旨を明記する。

## 1. 検証対象コミット

| 項目 | 値 |
|---|---|
| RC branch | `release-0-5-1` |
| 検証 head SHA | `21dd16c25a99ba6c8c4c36ed6e61f8ae130f9640` |
| RC コード SHA（PR #438 記載） | `fa1d82f`（#447 merge） |
| `fa1d82f..21dd16c` の差分 | **docs のみ**（#446 較正記録・#448 収束判断）。`crates/**` `ops/**` `Cargo.*` は無変更 = バイナリ同一 |
| 実行環境での build 先 | detached worktree `21dd16c`（共有 worktree を触らない分離構成） |

## 2. 実行環境

| 項目 | 値 |
|---|---|
| OS | Microsoft Windows 11 Home Insider Preview, 10.0.29683 (rs_prerelease.260930-1416), x64 |
| プロセス | Windows ネイティブ（WSL / Linux コンテナ / Docker 不使用） |
| PowerShell 5.1 | `C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe` — 5.1.29683.1000 |
| PowerShell 7 | pwsh 7.6.6 (Core) |
| rustc | 1.99.0 (b940084d7 2026-09-28), host `x86_64-pc-windows-msvc` |
| cargo | 1.99.0 |
| MSVC toolchain | Visual Studio 18 Insiders — `VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64\link.exe` |
| Windows SDK | `C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0` |
| git | 2.55.0.windows.5 |
| toolchain manager | **mise 2026.9.11**（`python` / `cargo` / `gh` / `node` / `bun` は mise shim 経由で解決） |
| mise tools (project) | `mise.toml` = bun 1.4.2 固定。`rust-toolchain.toml` = channel `stable` + rustfmt/clippy |
| mise tools (user) | python 3.14.8 / rust stable / gh 2.102.0 / node 26.10.0 / bun 1.4.2（`~/.config/mise/config.toml`） |
| python | 3.14.8（`C:\Users\rebui\AppData\Local\mise\installs\python\3.14.8\python.exe`） |
| ハードウェア | Intel Core i7-13700H (20 threads), RAM 32 GB, C: 空き 65 GB |

注: PATH 上の `link.exe` は coreutils の `link`（ハードリンクツール）であり MSVC の linker ではない。
実際のリンクは rustc が VS Insiders / Windows SDK を直接探索して行う。ビルド成功と実行確認で実在を実証済み。

## 3. ゲート結果（随時追記）

| Test | Result | SHA | Environment | Duration | Evidence | Notes |
|---|---|---|---|---|---|---|
| A. Native MSVC release build | **PASS** | `21dd16c` | Win11 / rustc 1.99.0 msvc | 1m43s | `gate-a-release-build.log`, `gate-a-procon-exe-sha256.txt` | warning 0 / error 0。`version` = `{"package":"0.5.1","protocol":{"major":1,"minor":0},"rustc":"1.85"}` |
| C. 模擬試合 (PowerShell 7) | **PASS** | `21dd16c` | pwsh 7.6.6 + `honsen.ps1` + release exe | 約40s | `gate-c-ps7-full.log`, `gate-c-honsen-start-ps7.log`, `gate-c-mock-summary.json`, `gate-c-run-manifest.json`, `gate-c-run-state.json`, `gate-c-record-show.json` | 3日全受理 `[0,1,2]`、`bad_token=0`、`missing_seq=0`/`unparsed_lines=0`、manifest SHA 一致、候補 `sized:budget=official:budget_ms=2000:convoy_ms=1000:seed=42` |
| D. Timeout / Recovery（#447 系） | **PASS** | `21dd16c` | Win11 / cargo test (MSVC) | 実測ログ参照 | `gate-de-protocol-cli-tests.log` | `agent_submit_recovery_e2e` 7/7（受理済みACK喪失→read-back 継続、kind 締切 ACK 喪失、setting timeout 越過、未達は unconfirmed、401/403 は read-back しない、復旧不能→`unconfirmed`、二重送信しない）+ `http_client` timeout/retry 系 + `accepted_posts_with_lost_acknowledgements_are_not_replayed` |
| E. tournament_basic (#391) | **PASS** | `21dd16c` | Win11 / cargo test (MSVC), PowerShell から実行 | 0.60s | `gate-e-tournament-basic.log` | `test result: ok. 6 passed; 0 failed`（`sh` 非依存・Windows hook 経路） |
| B. ZIP / offline bundle (PS5.1/7) | 未実施 | — | — | — | — | PR #443 の `verify-offline-bundle.ps1` を `fix/windows-offline-layout` から使用予定 |
| C. 模擬試合 (PowerShell 5.1) | 未実施 | — | — | — | — | 直前の失敗は `.env` のポート不整合（検証スクリプト側の不具合）。修正済み・再実行待ち |
| F. Telemetry (`PROCON_TELEMETRY=8437`) | 未実施 | — | — | — | — | #436 は release-0-5-1 未統合（branch `429` @ `63a1d0a` で実施予定） |
| 候補 A (sized) dispatch 境界 | 未実施 | — | — | — | — | `cargo test -p bench --test sized_candidate` + 実機 mock 試合 |

## 4. 再現手順（要点）

```powershell
# 1) SHA 固定
git fetch origin; git worktree add --detach <wt> 21dd16c25a99ba6c8c4c36ed6e61f8ae130f9640
$env:PROCON_SOURCE_COMMIT_SHA = (git rev-parse HEAD)
cargo build --release --locked -p cli
(Get-FileHash <wt>\target\release\procon.exe -Algorithm SHA256).Hash.ToLower()

# 2) ローカル mock + 3日模擬試合（本番 POST は行わない）
#    gate-c-run.ps1 が mock 起動 → honsen.ps1 start → summary/記録/HTTPログ/manifest を ASSERT する
pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File .\gate-c-run.ps1 -Port 18440
powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File .\gate-c-run.ps1 -Port 18451

# 3) ゲート D / E
cargo test --locked -p protocol -p cli
cargo test --locked -p cli --test tournament_basic
```

- 本番サーバー宛の通信は一切していない（アクセス先は 127.0.0.1 の `ops/honsen/mock_server.py` のみ）。
- 認証情報は `.env` のローカル用トークンのみで、記録ディレクトリ・HTTP ログへの書き込みをテストが検証している。

## 5. SHA256

```
gate-a-procon-exe-sha256.txt （対象: target/release/procon.exe @ 21dd16c）
```
