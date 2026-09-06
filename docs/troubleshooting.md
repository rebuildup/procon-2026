# Troubleshooting

遭遇しやすい失敗と復旧手順の一覧。Issue 化前の一次切り分けに使う。

## Rust

### `error[E0433]: cannot find 'fmt' in 'core'` が出る

自作 crate を `core` という名前にすると、`thiserror` などの derive macro が `::core::fmt` を std の `core` ではなく自作 crate 側と解決して壊れる。

**対処**: 自作 crate の名前を `domain` など stdlib `core` と衝突しない名前に変更する (当プロジェクトは `domain` を採用)。

### `cargo build` が stack overflow する

`rust-toolchain.toml` で stable / nightly 切替を確認。`RUSTC_BOOTSTRAP=1` は付けない。

### Lint 警告が大量に出る

`cargo clippy --fix --workspace --all-targets --allow-dirty --allow-staged` で機械修正可能なものを一括適用。残りは人手レビュー。

## Frontend

### WebGPU が "No GPU adapter available" で起動できない

- ブラウザフラグで WebGPU が有効か確認 (Chrome は `chrome://flags/#enable-unsafe-webgpu`)。
- headless / CI では WebGPU が動かない。`bun test` のような非ブラウザ環境では WebGPU 依存モジュールを import しないこと。

### `bun test` で WebGPU 関連モジュールを import して落ちる

`packages/frontend/src/lib/webgpu/*.ts` はブラウザ専用。`__tests__/` から直接 import せず、必要ならモックする。

### SolidJS の reactive が二重更新される

`createSignal` の setter に同じ値を渡しても write 扱いになる。重複排除は呼び出し側で行う。

## Protocol

### Rust と TypeScript で PROTOCOL_VERSION がズレる

CI の `protocol-consistency` job が失敗する。両者を更新して commit。

## CI

### Cache miss で初回が遅い

`actions/cache` / `Swatinem/rust-cache` の key を確認。lockfile 変更時に意図せず fail することもあるが、cache なしでも fail はしない。

### Dependabot が同時に大量の PR を出す

`dependabot.yml` の `open-pull-requests-limit` を下げる。長期的に ignore する major bump は `.github/dependabot.yml` で `ignore` リストに入れる。

## Git / GitHub

### `gh pr create` が "pull request already exists" で失敗

同じ branch から PR を作ろうとしている。`gh pr list --head <branch>` で既存 PR を確認。

### `git push` が rejected される

`gh repo edit --default-branch main` で main を保護している場合、release branch を直接 push する前に PR 経由で上げる。

### Worktree だけを使って isolated sandbox 代わりにした結果、port / DB がconflict する

worktree は Git 実装詳細であり、execution isolation ではない。必ず container / VM / process-level の isolation を組み合わせる。