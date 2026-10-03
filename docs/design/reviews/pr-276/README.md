# PR #276 — 画面確認と共通描画の検証

実装・録画 harness: `3541ffa`（product変更は `0356098`）。1280×720 / Chromium / SwiftShader / Noto Sans CJK JP。地図は問題文の例、Live入力はsnapshot fixture。実試合の地図を表示する録画ではない。

- [図1の中央表示](home.png)
- [図2とマップ外の空枠](sample-figure2-padding.png)
- [Solvers PNG](solvers.png) / [ズーム・中央に戻す・図2への切替・空枠追加の録画](solvers.webm)
- [Live PNG](live.png) / [録画](live.webm)
- [原資料の図1](official-figure1.png) / [原資料の図2](official-figure2.png)。`docs/reference/hexaudon_spec.html` の埋め込み原画像から抽出した。地形・スポット位置・系列・行のずれを再現し、描画素材は合流済みdesignを使う。
- [元SVG / GPU / Canvasの同座標比較](shared-renderer-comparison.png)
- [3 family / 15pxの6系列標本](families-15px.png)
- [描画計測](renderer-performance.json) / [撮影 manifest](manifest.json) / [Primer token 監査](primer-audit.json)

`bunx playwright test` で18件成功。録画シナリオに例/空枠の操作を追加後、`bunx playwright test e2e/record.spec.ts e2e/hex-design.spec.ts` の8件も成功。frontend check / lint / format / unit10件 / build成功。

GPU/Canvasは「全halo → 全背景/記号 → 全黒線」の順序で同じアトラスを描き、隣の塗りによる黒線欠けを防ぐ。六辺の黒画素・cell順反転の差・空枠付きcenterを検証する。GPUは1pipeline/1passの3 instanced draw、camera60回でatlas/instance再転送0。100cell原図1のCPU submission p95は計測JSONを参照（実機FPS保証とは区別する）。

日本語フォントは検証環境に追加したもの。productのPrimer system font指定は変更していない。詳細: [UI-275.md](../../validation/UI-275.md)。
