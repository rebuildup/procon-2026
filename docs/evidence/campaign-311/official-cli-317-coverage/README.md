# 公式バイナリ × 実CLI smoke — coverage:budget=official / 701

source: `0eb3a9f6e91ada406f6d1cff4d65e3c60c9fe7b9`。公式配布サーバー (sha256 `cd552d93…`) と release build の `target/release/procon` (sha256 `8249b900…`) をローカルで接続し、実CLI coordinator 経由で `coverage:budget=official` を 701 盤・10 日・`P,S`（5 台）で最後まで駆動した。

- `POST /agent` で kinds 受理、以後 `POST /` の提出が 10 日すべて `revision=1..10` で受理、`--exit-after-match` で exit 0。
- daySteps は 701 の公式値、daySeconds は 4 秒に短縮。1 チームのみ。公式形式の提出が最後まで通ることの確認であり、**arena スコア一致ではない**。
- `requests.json` に全 HTTP 往復（`GET /setting`、`POST /agent`、`GET /`、`POST /` の提出計画）、`cli.log` に coordinator 出力、`server.log` に公式サーバー出力、`config.json` に生成した match 設定、`summary.json` に source/binary digest と revisions。

再現: `python3 scripts/campaign/official_cli_smoke.py --server-binary <公式サーバーバイナリ> --candidate 'coverage:budget=official' --kinds 'P,S' --output-dir <空のディレクトリ>`。

未完: 外部 arena 未接続。スコア・順位の検証ではない。#317 の全受け入れ条件完了を意味しない。
