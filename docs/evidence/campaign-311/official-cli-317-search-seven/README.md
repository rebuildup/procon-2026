# 公式バイナリ × 実CLI smoke — search 16 trials / 7台低燃料

source: `0eb3a9f6e91ada406f6d1cff4d65e3c60c9fe7b9`。公式配布サーバー (sha256 `cd552d93…`) と release build の `target/release/procon` (sha256 `8249b900…`) をローカルで接続し、実CLI coordinator 経由で `search:budget=official:trials=16` を 701 盤・10 日・`P,P,P,S,P,P,P`（7 台）で最後まで駆動した。

- `--seven-agents` 設定: 701 の agentStarts を 7 台に複製し fuelLimits を 12 に制限、daySeconds は 4 秒に短縮。
- `POST /agent` で 7 台の kinds 受理、`POST /` の提出が 10 日すべて `revision=1..10` で受理、`--exit-after-match` で exit 0。
- 短縮 window・低燃料・7 台の負荷下でも search の live 回答窓制御が最後まで提出を続けたことの確認。day movement は待機中心（1 日あたり moves 0〜119）。**スコア・順位の検証ではない**。
- `requests.json` に全 HTTP 往復、`cli.log` に coordinator 出力（`offline candidate selected` 行に candidate id）、`server.log` に公式サーバー出力、`config.json` に生成した match 設定、`summary.json` に source/binary digest と revisions。

再現: `python3 scripts/campaign/official_cli_smoke.py --server-binary <公式サーバーバイナリ> --candidate 'search:budget=official:trials=16' --kinds 'P,P,P,S,P,P,P' --seven-agents --output-dir <空のディレクトリ>`。

未完: 外部 arena 未接続。candidate id は `search:budget=official:trials=16`（評価表の `search:trials=16:budget=official` と同設定・表記違い）。#317 の全受け入れ条件完了を意味しない。
