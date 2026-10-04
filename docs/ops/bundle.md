# 提出物バンドル (`procon bundle build`)

予選 / 本選の提出物（システム本体 + ドキュメント + 動作設定例）を 1 つの
zip にまとめて再生成する仕組み。

実装は `crates/cli/src/bundle.rs`、CLI エントリは `procon bundle build <output.zip>`。

## 1. 使い方

```bash
# ワークスペースルートで実行
cargo build --release --bin procon --locked

procon bundle build dist/procon-submit.zip
```

`bundle build` は毎回 `cargo build --release --bin procon --locked` を実行し、
Cargo の freshness 判定を通す。実行ファイルは Cargo の JSON 出力から解決するため、
`CARGO_TARGET_DIR` や platform の拡張子にも対応する。実行ファイルの欠落は失敗とする。

## 2. zip に含むファイル

| アーカイブパス | 元ファイル | 用途 |
| --- | --- | --- |
| `procon` | `target/release/procon` | 提出用実行バイナリ |
| `README.md` | `README.md` | プロジェクト概要 |
| `docs/contest/strategy.md` | `docs/contest/strategy.md` | 並走トラック独自戦略 |
| `docs/contest/team-context.md` | `docs/contest/team-context.md` | 並走トラック背景 |
| `docs/contest/api.md` | `docs/contest/api.md` | プロトコル API リファレンス |
| `.env.example` | `.env.example` | 環境変数テンプレート |

任意の文書ファイルが存在しない場合は警告を `stderr` に出してスキップする（CI で個別
ファイルが削除されていても build 自体は成功する）。

## 3. zip フォーマット詳細

- 圧縮: deflate (`zip` crate の `CompressionMethod::Deflated`)
- ファイル権限: `0o644`（Unix 系 OS で `unzip -l` した時に自然な表示）
- エントリ名: `\\` を `/` に正規化（Windows クロスコンパイルでも動く）
- 文字エンコーディング: UTF-8（CP932 環境の unzip でもファイル名は扱える）

## 4. 機密情報 lint

`build_bundle` を呼ぶ前に `lint_secrets` で下記を検査する。
1 件でも該当したら `error: ...` でビルドを中断する。

### 4.1 禁止ファイル名 (basename 一致)

- `.env`（実体の env ファイル）
- `id_rsa`, `id_ed25519`
- `*.pem`, `*.key`

### 4.2 内容パターン (regex)

| ラベル | パターン |
| --- | --- |
| `api_key_assignment` | `api_key = <16文字以上>` 等 |
| `secret_assignment` | `secret = <16文字以上>` 等 |
| `token_assignment` | `auth_token = <16文字以上>` 等 |
| `password_assignment` | `password = <8文字以上>` 等 |

- バイナリ（UTF-8 でない）エントリはスキャン対象外（実行バイナリ自体が該当
  しないようにするため）。
- `.env.example` は値が無いので false-positive は出ない（テストで確認済み）。

## 5. stdout 出力フォーマット

```text
bundle written: dist/procon-submit.zip
files:
  - procon                                       12345678 bytes
  - README.md                                    1234 bytes
  - docs/contest/strategy.md                     5678 bytes
  - docs/contest/team-context.md                 2345 bytes
  - docs/contest/api.md                          3456 bytes
  - .env.example                                 234 bytes
totals: 6 entries, 12359925 uncompressed bytes, 4567890 on disk
```

CI で grep しやすいよう、1 行 1 情報のフォーマットを維持する。

## 6. CI / 提出フローへの組み込み

`just ci` (`fmt-check` + `lint` + `test` + `build`) 通過後にバンドル生成
ジョブを 1 段追加する想定。

```yaml
- name: Build submission bundle
  run: |
    cargo build --release --bin procon --locked
    procon bundle build dist/procon-submit.zip
- uses: actions/upload-artifact@v4
  with:
    name: procon-submit
    path: dist/procon-submit.zip
```

## 7. 拡張余地

- `procon bundle inspect <path>` — 中身一覧とサイズを JSON で出力
- `procon bundle verify <path>` — ビルド時 zip と再生成 zip の diff
- 提出アップロードは本スコープ外 (Issue #52 Out of scope)