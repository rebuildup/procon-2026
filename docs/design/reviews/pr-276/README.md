# PR #276 — 画面確認と共通描画の検証

実装・録画元: `d528ebb`（product変更は `5cdc8c6`）。1280×720 / Chromium / SwiftShader / Noto Sans CJK JP。マップは問題文の例、Live入力はsnapshot fixture。実試合の地図を表示する録画ではない。

- [マップ構成例（募集要項p.8）](home.png) / [録画](home.webm)
- [スポット配置例（募集要項p.10）と外側2セル分の空枠](sample-figure2-padding.png)
- [Replays PNG](replays.png) / [録画](replays.webm)
- [Solvers PNG](solvers.png) / [例切替・ズーム・Reset view・空枠追加の録画](solvers.webm)
- [Live PNG](live.png) / [録画](live.webm)
- [原資料p.8の図1](official-figure1.png) / [原資料p.10の図2](official-figure2.png)。埋め込み原画像から抽出。地形・位置・系列を再現し、絵柄は合流済みdesignを使う。
- [元SVG / GPU / Canvas比較](shared-renderer-comparison.png) / [3family・15px標本](families-15px.png)
- [計測](renderer-performance.json) / [撮影manifest](manifest.json) / [Primer監査](primer-audit.json)

共通atlas内のhexを0.93倍に縮小し、各cellの六辺の黒線と隣接cell間の白い余白を保つ。GPU/Canvasとも「全halo → 全背景/記号 → 全黒線」の順序を使う。六辺の黒画素・隙間の白画素・描画順反転・中央合わせを検証する。GPUは1pipeline/1passの3 instanced draw。camera60回でatlas/instance再転送0。CPU submissionの計測は実機FPS保証とは区別する。

平地・道路・セル数・巡回車など問題由来の用語は日本語に合わせ、Play / Pause / Reset viewなどviewer独自の操作名は英語にする。出典は正式なPDFファイル名・ページ・図番号とリンクを表示する。[公開ルール](https://rebuildup.dev/share/procon2026)の地形・エージェント・スポットと原資料p.8/p.10への5リンクは実ブラウザで到達確認済み。HeaderのRulesも関連セクションへ直接移動する。

frontend check / lint / format / unit10件 / build成功。ブラウザ19件成功（4画面の録画を含む）。Primerの独自/未定義style tokenは0。日本語フォントは検証環境に追加し、productのPrimer system fontは変更していない。詳細: [UI-275.md](../../validation/UI-275.md)。
