# docs/reference — 参考資料

公式資料そのものではない、外部入手・派生の参照用アーティファクトを置く。
公式原本は `docs/pdf/`（正本は team-context.md の方針どおり原本PDF）。

## hexaudon_spec.html — ヘキサうどん｜競技仕様ガイド

単一HTMLの仕様ガイド。第37回高専プロコン競技部門について、仕様整理・公式Q&A 62件・
原資料6件（全31ページ）を1ファイルに集約している。

- **閲覧**: ブラウザで `hexaudon_spec.html` を直接開く。CSS / フォント / 画像 / PDF を
  base64 で内包しているため、ネット接続なしで全内容を閲覧できる。
- **構成**: 仕様目次（13章）→ 各項目から Q&A・原ページへの出典リンク / Q1〜Q62 の番号ジャンプ /
  原資料6本（base64 内包の「原PDFをダウンロード」リンク。インライン表示ではない）。
- **収録**: PDF 6本 + PNG 35点を base64 内包。ファイルサイズは約 16 MB。
- **由来**: 手元のダウンロードファイル `hexaudon_spec.html` を 2026-10-02 に無改変でコピー。
  作成者・配布元の明記はファイル内にない（末尾の記載はデザイン参考のみ）。

### 正本との関係

本ガイドは添付資料を再整理した**派生資料**であり、正本ではない。
収録PDF自体は原本と同一でも、要約・並べ替えはガイド側の編集である。

- 正本: `docs/pdf/` の原本PDF（募集要項 / フォーマット / Q&A 各弾）
- 判断に使う仕様は、ガイドで当たりをつけたあと原本PDFで確認する
- `docs/contest/team-context.md` の「公式資料は原本PDFを正とする」方針は本ファイルでも変更しない

## official-md/ — 公式資料の Markdown 写経（参照用）

cslab-nituc/procon2026 の `docs/src/official/{guideline,q-1}.md` を無改変コピー。
勝敗判定4tier・地形/燃料/在庫/交通ルール・Q1–Q21 のパラメータ範囲を全文検索できる参照用。
**正本は `docs/pdf/` の原本PDF**（写経との齟齬があれば原本が正）。`@@CONFIDENTIAL@@` 継承。
詳細と出典 commit は `official-md/README.md`。

## procon2026-diary/ — 開発日記 2026-04-26（参照用・CONFIDENTIAL）

cslab-nituc/procon2026 の戦略考察頁（loop-tour・系列カバレッジ優先・逆方向補給ペア案）。
**着想の参照のみ**、本リポの文書へは転載しない（team-context.md 並走ルール）。
詳細は `procon2026-diary/README.md`。

## live-match/ — 公式サーバー実演のプロトコル録画

公式配布サーバーをローカルで動かした2チーム4日分の実 HTTP 応答・提出プラン（Issue #309）。
protocol/wire 形式の参照と `arena import-case` の可否調査に使う。詳細は `live-match/PROVENANCE.md`。

## 取扱注意（Issue #309 / 2026-10-08）

上記3ディレクトリは `docs/contest/team-context.md` §CONFIDENTIAL フラグの扱い 3.
（原著作者クレジット + `@@CONFIDENTIAL@@` 明示）に従って取り込んだもの。
判断の正本は変わらず `docs/pdf/` の原本PDFと `docs/contest/` の方針。
