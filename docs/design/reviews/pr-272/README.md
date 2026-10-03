# PR #272 — 画面確認用の録画とスクリーンショット

実装 commit: `15c794b6d78ed5218d66ce632cf4a16f025179a9`。1280×720、Chromium 153。`bunx playwright test e2e/record.spec.ts` で4件とも成功した録画を保存した。source の変更は含まない。

| 画面 | スクリーンショット | 録画 | 動作 |
|---|---|---|---|
| Map | [PNG](home.png) | [WebM](home.webm) | visualizer 主領域と fixture / legend パネル |
| Replays | [PNG](replays.png) | [WebM](replays.webm) / [GIF](replays.gif) | 再生・score・速度2×、末尾到達後の再生 |
| Solvers | [PNG](solvers.png) | [WebM](solvers.webm) / [GIF](solvers.gif) | WebGPU のズーム・パン・Reset view |
| Live | [PNG](live.png) | [WebM](live.webm) | schemaVersion 1 の snapshot fixture を polling |

GIF は WebM の0.5秒から6.25秒を250ms間隔で抽出し、縮尺を変えずに符号化した PR 内プレビュー。全録画は WebM を参照する。動画・画像の取得時刻、サイズ、browser/GPU probe は [manifest.json](manifest.json) にある。

地図・replay は demo、Live の snapshot はテスト fixture。実試合の録画ではない。private repository 内の review evidence として保存し、PR 本文はこの evidence commit に固定したリンクを使う。

コンポーネント監査: [primer-audit.json](primer-audit.json)。再生操作は表示ラベル付き Button / BtnGroup、速度はラベル付き radio group。監査と light/dark computed-style 比較の詳細は [UI-271.md](../../validation/UI-271.md)。GitHub.com 全画面との pixel 完全一致を示す資料ではない。

地形・境界・全12役割の地図記号・agent は同じ WebGPU シーンで描画する。記号は共有アトラスに一度だけ変換し、地図内に SVG DOM を置かない。GPU 不可時は同じアトラスで Canvas 描画へ退避する。Visualizer は Primer utility の枠でセクション化し、Reset view は View パネルに置く。
