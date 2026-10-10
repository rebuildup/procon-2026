# ヘキサうどん戦略ラボ（参考HTML）

Issue: [#342 — 戦略ラボの手法を公式ソルバへ検証導入する](https://github.com/rebuildup/procon-2026/issues/342)

## ファイル

- [hex-udon-strategy-lab.html](./hex-udon-strategy-lab.html) — ユーザーから提供された単一HTML（盤面生成／戦略シミュレータ／探索／可視化）
- **原本を無改変で保存**。このフォルダは参考資料置き場であり、公式エンジンや本番ソルバには接続していない。

HTMLは独立したシミュレーション実装を持つ。以下の機能が参考になる。

| 機能 | 関連関数 |
|---|---|
| 貪欲＋ランダム試行 | `greedyDay`, `bestDay` |
| ビーム探索＋ロールアウト | `beamDay`, `bestDayBeam` |
| 翌日・翌々日の見積もり | `bestDayBeam`, `forecastTraffic` |
| 巡回・補給の方針 | `computePlanCfg`, `loopSupply`, `buildLoop`, `hubCandidates` |
| 1日の実行トレース・損失分類 | `analyzeDay` |
| 複数の盤面・戦略比較 | `runMatch`, バッチ比較UI |

## 取り込み方針

1. まず公式Rust `engine::MatchSim`／wireプロトコル／`normalize_day_plan`とのルール差分を調査する。HTMLの `simulate` や `trafficFor` を公式評価器と見なさない。
2. アルゴリズムの**着想**を独立Candidateに実装し、`coverage:budget=official:park=far` 等の既存基準と同一条件で比較する。
3. 公式の `variety → daily_variety_sum → total_balls → response_time` に沿った辞書順で評価する。加重近似値やHTMLの合計スコアで採用を決めない。
4. 新Candidateはholdout、release profile、offline/live parity、燃料・期限・行動正規化を検証し、Issue/PR回収担当へ送る。外部ArenaはArena専任セッションのみ操作する。
5. 可視化の移植は既存の [#308](https://github.com/rebuildup/procon-2026/issues/308) と重複調査し、ソルバ改善を優先する。

## 来歴と権利

2026-10-09、ユーザーから本作業用にアップロードされたHTMLを原本のまま保存したもの。**作者・ライセンス・第三者コードの権利関係は未確認**であり、この保存によってリポジトリの他のコードと同じOSSライセンスが確認されたわけではない。現在は非公開リポジトリ内の研究参考資料として管理する。

一般公開・別製品への再配布・ソースの直接流用に先立ち、出所と利用条件を確認する。HTML内で使うGoogle Fonts等の外部リソースは別途その利用条件に従う。
