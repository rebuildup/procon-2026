# PR #272 — 画面確認用の録画とスクリーンショット

実装 commit: `7f135e04891b236d17782dd5b494518aec2a8979`。1280×720、Chromium 153。`bunx playwright test e2e/record.spec.ts` で4件とも成功した録画を保存した。source の変更は含まない。

| 画面 | スクリーンショット | 録画 | 動作 |
|---|---|---|---|
| Map | [PNG](home.png) | [WebM](home.webm) | visualizer 主領域と fixture / legend パネル |
| Replays | [PNG](replays.png) | [WebM](replays.webm) / [GIF](replays.gif) | 再生・score・速度2×、末尾到達後の再生 |
| Solvers | [PNG](solvers.png) | [WebM](solvers.webm) / [GIF](solvers.gif) | SVG → WebGPU → SVG |
| Live | [PNG](live.png) | [WebM](live.webm) | schemaVersion 1 の snapshot fixture を polling |

GIF は WebM の0.5秒から6.25秒を250ms間隔で抽出し、縮尺を変えずに符号化した PR 内プレビュー。全録画は WebM を参照する。動画・画像の取得時刻、サイズ、browser/GPU probe は [manifest.json](manifest.json) にある。

地図・replay は demo、Live の snapshot はテスト fixture。実試合の録画ではない。private repository 内の review evidence として保存し、PR 本文はこの evidence commit に固定したリンクを使う。
