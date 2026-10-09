# 公式資料の Markdown 写経（参照用）

<!-- @@CONFIDENTIAL@@ 継承: 元文書に大会終了までの取扱注意マークあり -->

**正本は `docs/pdf/` の原本 PDF。本ディレクトリの写経は検索・引用のための参照用であり、**
**仕様判断で齟齬が出たら必ず原本 PDF で確認する**（`docs/contest/team-context.md` の方針どおり）。

## 出典（provenance）

- Source: <https://github.com/cslab-nituc/procon2026> / branch `main` / commit `04da2ff`
- 取得日: 2026-10-08（ファイル無改変コピー、SHA-256 一致確認済み）
- 原文パス: `docs/src/official/guideline.md`（募集要項の写経）、`docs/src/official/q-1.md`（Q&A 第1弾 Q1–Q21 の写経）
- 原著: 第37回高専プロコン 競技部門 募集要項 / 問い合わせと回答（原本 PDF は `docs/pdf/`）

## @@CONFIDENTIAL@@ について

両ファイル先頭には元文書由来の `@@CONFIDENTIAL@@` マークがある（大会終了までの取扱注意）。
本リポジトリ（private）へは `docs/contest/team-context.md` §CONFIDENTIAL フラグの扱い 3. の手順
（原著作者クレジット + フラグ明示）に従って取り込んだ（Issue #309, 2026-10-08 決定）。
マークは削除していない。

## 主要な参照ポイント

- 勝敗判定の優先順位（系列数 → 日別系列数の累積 → 玉数 → 回答時間累積 → サイコロ）: `guideline.md`
- 地形ごとの時間/燃料、在庫・補給・交通量の日次ルール: `guideline.md`
- パラメータ範囲（混雑1-5 / 渋滞2-10、燃料 1-3×日目ステップ、日ステップ W+H..4(W+H) 等）: `q-1.md`
- 行動受理の処理順序（Q6）と計画全体リジェクト条件: `q-1.md`
