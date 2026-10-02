# 録画エビデンス — Issue #258 / PR #260

Playwright 録画ハーネス（`just record`）で撮影した **PR #260 のレビュー用 成果物**。
通常の録画出力は `target/recordings/`（gitignore 済み）で、実行のたびに作り直される。
このフォルダは当時の実行結果を PR レビューのために固定したもの。

| file | 内容 |
|---|---|
| `home.{png,webm}` | `/` |
| `replays.{png,webm}` | `/replays` — Play → 2× → 終端で巻き戻して再再生 |
| `solvers.{png,webm}` | `/solvers` — SVG → WebGPU → SVG の切替 |
| `live.{png,webm}` | `/live` — `/match-state.json` を committed fixture で route 補完 |

- 撮影環境: headless chromium 153（WebGPU = SwiftShader、`manifest.json` の `webgpu` 参照は当時の `target/recordings/manifest.json`）
- 再生成: `just record`（手順は [`docs/development.md`](../../development.md) § 画面録画）
- 既知の視覚バグ: WebGPU canvas が headless で白紙 → Issue #259（本フォルダの `live.png` / `solvers.webm` に現れている）
