# live-match 録画の出典（provenance）

公式配布の簡易サーバー（`procon-server-linux-amd64`）をローカルで起動し、2チーム4日間の
対戦を実行したときの**実 HTTP 応答と提出プランの録画**です。大会・アリーナの実戦ログではなく、
プロトコル連携の参照用素材として取り込みます。

- Source: <https://github.com/cslab-nituc/procon2026> / branch `kimura-visualizer` / commit `b711222`
- 原文パス: `docs/images/live-match/`
- 取得日: 2026-10-08（GitHub Contents API で取得、バイト数一致確認 + JSON parse 確認済み）
- サーバーバイナリの SHA-256 は `source.json` に記録（トークン・認証情報は元から不含）

## ファイル

| ファイル | 内容 |
|---|---|
| `match-data.json` | `GET /setting` と各日 `GET /` の実応答（4日分） |
| `team-0-plans.json` / `team-1-plans.json` | 両チームが `POST /` で提出したプランと受理応答（revision） |
| `source.json` | 録画条件（サーバー SHA-256・入力設定のハッシュ等） |
| `manifest.json` | 同時録画のスクリーンショット/動画の目録 |
| `README.md` | 上游の録画説明（原文のまま） |

## 使い方

- protocol / wire 形式の参照（計画配列 `[dir>=0 | -wait]`、`revision` 応答、`players` 等）
- Issue #309 の追加調査: `just arena-import`（`arena import-case`）が `match-data.json` を
  eval 盤として取り込めるか fail-closed で試すこと（不通なら本資料は raw 参考のまま）

画像・動画（`.png/.gif/.webm`）は本 Issue のスコープ外のため取り込んでいません。
