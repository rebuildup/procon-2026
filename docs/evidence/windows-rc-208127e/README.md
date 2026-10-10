# Windows 11 ネイティブ RC 検証 — `208127e`（release-0-5-1 @ `1dfd603` = #460 sized 既定化を含む + #443 + #449）

実行日: 2026-10-11 JST / ブランチ: `443-verify-rc` / ソース SHA: `208127e3f9b88bca62e096913ea2fb2e9931d8fa`
exe SHA256: `C562487D2C1AD90A3E07FA47820F057B69DAED905E3FF58069511EE4EE21356B`（manifest の `binary.sha256`・展開 ZIP の exe と一致）
環境: Windows 11 10.0.29683 / PS 5.1.29683.1000 + pwsh 7.6.6 / rustc 1.99.0 msvc / Python 3.14.8。
**本番サーバーには接続していない。** 模擬サーバーは 127.0.0.1 のみ。先行証跡 `windows-rc-21dd16c` / `windows-rc-0a1fa5d` とは別 SHA。

## 判定

| # | 項目 | 判定 | 証跡 |
|---|---|---|---|
| 1 | release build / version / source SHA / exe SHA256 / manifest 整合 | PASS | `verify-rc-build-release.log`, `verify-rc-sha-check.log`, `s*-manifest.json` |
| 2 | PS 5.1 / PS 7.6 の preflight・start・show | PASS | `verify-rc-bundle-e2e.log`（5.1）、`s24-*`（pwsh 7）、`s16-*` `s32-*`（5.1） |
| 3 | ZIP 生成 → 別ディレクトリ展開 → オフライン起動（3 日受理） | PASS | `verify-rc-bundle-e2e.log`, `verify-rc-e2e-mock.log`（ZIP は `honsen.ps1` と `procon.exe` のみ） |
| 4 | 16×16 sized→convoy、全日受理 | PASS（2 日・mock） | `s16-*`: 提出は日開始から 6.1 s / 4.9 s（convoy_ms=20000 上限内で収束）、2 日受理 |
| 5 | 24×24・32×32 sized→fleet、回答 margin、全日受理 | PASS（各 2 日・mock） | `s24-*`: 提出 54.7 s / 54.7 s、`s32-*`: 54.6 s / 54.6 s（60 s 窓、余裕 約 5.3〜5.4 s）、全日 200 受理 |
| 6 | 二重起動拒否 / 異常終了後の復帰 | PASS | `verify-rc-double-start.log` |
| 7 | records・HTTP log・revision・全日受理の保存 | PASS | `s*-http.jsonl`, `s*-events.jsonl`, `s*-manifest.json`, `s*-mock-summary.json`（accepted_days=[0,1], bad_token=0） |
| 8 | 既定値と手動指定値の一致 | PASS | `honsen.ps1` 既定・`build-release.sh`・runbook・実行ログの candidate が全て `sized:budget=official:budget_ms=59000:convoy_ms=20000:seed=42:fuel_need=0`、`--kinds-auto`（`s*-start.log` で確認） |
| 9 | token・.env を ZIP / Git 証跡に含めない | PASS | ZIP は 2 ファイルのみ。証跡に `.env` なし、token は検出されず（mock の `local-e2e-token` は mock 用で秘密ではない） |
| - | 公式簡易サーバー E2E | BLOCKED（verify-rc は SKIP） | バイナリが実機に無い |
| - | 本番サーバー接続・実試合提出 | BLOCKED | 利用不可 |
| - | telemetry（#436） | SKIP | 未統合（別 RC） |

verify-rc 総合（`summary.txt`）: PASS=7 / FAIL=0 / SKIP=2。FAIL・修正なし（探索コード・本番戦略は未変更）。

## 注意（FAIL ではない所見）
- dispatch は candidate ログに明示されない。16×16 と 24/32 の提出時刻の差（約 5 s と約 54.6 s）と、32×32 の kinds が `S,P,...`（kinds-auto）になることから判断した。
- `ops/honsen/e2e.py` の `--candidate` 既定と `ops/arena-runner`（`fleet:...budget_ms=10000`）は旧既定のまま。本番起動経路ではなく Arena/公式サーバー用ツールなので変更していない。
- 日あたり 60 s 窓は mock の設定値。公式サーバーの実際の締切・受理は未検証。
- 各盤面 2 日のみ（10 日全日・複数盤面の締切余裕は #453 等の記録に依存）。
