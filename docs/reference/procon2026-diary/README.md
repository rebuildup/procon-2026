# procon2026 開発日記（参照用・CONFIDENTIAL）

<!-- @@CONFIDENTIAL@@ 継承: 元文書に大会終了までの取扱注意マークあり -->

**他トラックの戦略考察資料。本リポジトリの文書はここを転載せず、あくまで着想の参照に留め、**
**本リポ独自の分析として書く**（`docs/contest/team-context.md` 並走ルール「戦略は独立に再考」）。

## 出典（provenance）

- Source: <https://github.com/cslab-nituc/procon2026> / branch `main` / commit `04da2ff`
- 取得日: 2026-10-08（ファイル無改変コピー、SHA-256 一致確認済み）
- 原文パス: `docs/src/04/26.md`（mdBook「第37回高専プロコン開発記」2026-04-26 の頁）
- 原著: 並走トラックの開発者（同 mdBook の著者）

## @@CONFIDENTIAL@@ について

先頭に元文書由来の `@@CONFIDENTIAL@@` マークあり。`docs/contest/team-context.md`
§CONFIDENTIAL フラグの扱い 3.（原著作者クレジット + フラグ明示）に従い Issue #309 で
取り込み（2026-10-08 決定）。マークは削除していない。

## 何を参照するか（solver 改善キャンペーン用）

- **loop-tour の設計考察**: 全スポットを含む必要は無いが、**系列（種類）カバレッジは勝敗判定の
  最優先**なので全系列を回す価値が高い、という方針（＝本リポの evaluator 4-tier の tier1/tier2 と整合）
- **逆方向巡回ペア**: 巡回車と補給車を反対方向に走らせると1周で2回合流し、燃料予算が
  `2 × 最大積載量 × 補給車数` になる、という補給連携案（Issue #252 の escort 改善の着想源）
- 遠いスポット除外で密度を上げる / 経路は動的・順序は固定、などの tour 選択基準

実装の判断は本リポの `docs/contest/strategy.md` と evaluator 計測で行う。
