# #312 初動 baseline

source: `6bb166b99b227a5a3d20558637c3d3c067586db6`。debug profileで取得した探索用結果。回答時間をrelease本番性能と解釈しない。

固定9盤、arena701、由来未確定のimport盤、generated3盤の14盤を4候補×2車種構成で比較。全112件scored、invalid/timeout/panicなし。各runの公式4段scoreと車両別玉数は各 `runs.jsonl`、実行コマンドと環境は `environment.json`。

701 / P,S: grid2x2=20/119/173、kmeans5=20/158/207、kmeans8=20/148/200、loop128=18/139/189。kmeans5は701で優位だが、全14盤ではgridに5勝3分6敗で全盤優位ではない。

701 / P,S,S,P,S: grid2x2=17/90/119、kmeans5=20/144/169。車種が異なるbatch間の数字を候補改善として比較しない。

追加importは `fixture:arena-20261008T143339Z`（10×10、4日、3台、fuel20）。match696ではない。match696の14/70/320は既存Issueの相手実績のみで、現条件で再現できていない。

再現: `python3 scripts/campaign/baseline.py --kinds P,S --output-dir target/baseline-new`。本番用時間計測は `--profile release`。既存outputを上書きしない。

未完: 日別取得種類集合と停止原因の追加診断、release profile、holdout、新候補比較、live parity。#312の全受け入れ条件完了を意味しない。
