# PR #276 — hex デザイン合流の画面確認

実装: `66443023beaab3c1cfdacd10605e828e814984f1`。1280×720 / Chromium / SwiftShader WebGPU。Solvers・Live の地図は fixture。実試合ではない。

- [Solvers PNG](solvers.png) / [録画](solvers.webm)
- [Live PNG](live.png) / [録画](live.webm)
- [3 family の GPU 表示と15px隣接6系列](families-15px.png)。各地図の下の系列行が15px標本（拡大していない）。この比較用ページは e2e 内だけで生成し、product の DOM に記号一覧は追加しない。
- [撮影 manifest](manifest.json) / [Primer token 監査](primer-audit.json)

`bunx playwright test` で canonical 36記号一致・GPU3family・15px標本・従来UI・Primer参照比較・録画をまとめて確認。基準 proposal HTML は変更していない。詳しい色差分は [UI-275.md](../../validation/UI-275.md) を参照。
