# 公式サーバーバイナリ tick 検証 — refuel 補給誤判定の修正確認

source: `8728ed5d421947d477e11fdcba5cfdcb085467db`（`fix(solver): normalize refueling on official terrain ticks`）。
公式配布サーバー (`procon-server-linux-amd64` と同一 sha256 `cd552d93…`) をローカル起動し、旧 normalizer が許可する回答が公式受理されず、修正版 prefix が受理される最小ケースを確認した。

8×8 盤、補給点 pos=2、agent は平地 pos=0 の巡回車と道路 pos=2 の補給車、fuel=2、1 日 4 步。

1. `POST /agent [0, 1]` → 200。
2. `GET /` で両車とも fuel=2。
3. `POST / [[2,2,-1],[5,5,-2]]`（旧 normalizer の許可例・要素時刻の接触補給を計上）→ `{"revision": -1}` で拒否。
4. `POST / [[2,-2],[5,5,-2]]`（移動を出発地の地形コスト後に反映した修正版）→ `{"revision": 1}` で受理。

公式 tick では存在しない接触補給を旧 normalizer が計上していたことを、公式バイナリの受理判定で確認した。`result.json` に全リクエストと応答、`config.json` に盤面、`server.log` にサーバー出力を保存。

再現: `python3 scripts/campaign/official_tick_probe.py --server-binary <公式サーバーバイナリ> --output-dir <空のディレクトリ>`。

未完: 外部 arena での再確認ではない。本ディレクトリは normalizer 修正の公式受理検証であり、スコア比較ではない。#317 の全受け入れ条件完了を意味しない。
