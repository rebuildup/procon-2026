# チームリポジトリへのGit Subtree同期（GitHub Actions）

## 目的

個人研究 `rebuildup/procon-2026` の `main` を、チームの非公開 `cslab-nituc/procon2026` の既存PR #26（ブランチ `link-personal-research`）へ `research/procon-2026/` として投入する。`git subtree --squash` の正規の履歴を残す。初回はadd、以降はpull。チームの `main` を直接変更しない。

Workflow: `.github/workflows/sync-research-to-team.yml`。**手動起動のみ**。個人側 `main` にこのworkflowがマージされるまで、GitHub UIのRun workflowからは起動できない。

## 認証（最初に必要）

1. 個人側の GitHub Actions `GITHUB_TOKEN` は個人リポジトリの読取に使う。別のOrganizationリポジトリには書き込めない。
2. チーム側 `cslab-nituc/procon2026` への書込権限を持つ **GitHub App installation token** または承認済みの適切なPATを、個人側リポジトリのActions Secret **`PROCON_TEAM_WRITE_TOKEN`** として用意する。ワークフローが利用する資格情報はチーム側のGit clone / 非force pushに必要な最小権限とする。
3. Fine-grained PATを使える場合は、リソースオーナーを `cslab-nituc`、選択リポジトリを `procon2026` のみに限定し、`Contents: Read and write` を付与する。組織の承認設定によってはOwnerの承認が必要。
4. **外部CollaboratorではFine-grained PATが対象Organizationを選択できない場合がある。** その場合はOwnerにOrganizationメンバーとして追加してもらうか、対象リポジトリへインストールしたGitHub Appの権限を使う。Classic PAT（広いrepo権限）は最小権限にできないので推奨しない。
5. GitHub Appトークンは短命。GitHub Appsを用いる場合は、別の安全な認証発行手順（App ID/秘密鍵をActions Secret等から読み込み、actions/create-github-app-tokenで短命トークンを生成）に合わせてworkflowの`token:`部分を変更する必要がある。現状のworkflowは**Secretに格納された既存の書込トークンを直接使用するPAT向け構成**であり、GitHub Appトークンの自動発行は未実装。

Secret登録先：`https://github.com/rebuildup/procon-2026/settings/secrets/actions`（GitHub Settings → Secrets and variables → Actions）。Secret値をIssue/PR/ログや会話へ貼らない。

## 初回実行

1. PRを個人側のreleaseブランチに統合し、その後、正式なrelease PR経由で `main` へ反映する。GitHubの `workflow_dispatch` はデフォルトブランチにworkflowファイルが必要。
2. チーム側PR #26がまだDraftで、 `link-personal-research` ブランチが存在し、`research/procon-2026/` が未追加であることを確認。
3. 個人側 `main` に含まれる全追跡ファイルがチーム内に複製可能であることを確認。PDF、ZIP、動画も含まれる。機密情報や共有条件に問題がある場合は実行しない。
4. Actions → **Sync research subtree to team PR** → Run workflow。`approve_full_copy=true` を明示する。必要なら `expected_source_sha` に確認済みコミットSHAを指定し、元のmainが更新されていたら停止させる。
5. 成功時、チーム側 PR #26 に正規のSubtreeコミットが追加される。チームPRはDraftのままなので、変更をレビューして検証後にReady化・マージする。

## 実行が行う安全確認

- 実際の個人 `main` のコミットSHAを記録し、指定SHAとの一致を検証。
- 明白な秘密鍵・実 `.env` の追跡ファイル名があれば停止（**ファイル内容の機密監査の代わりにはならない**）。
- チーム側 `link-personal-research` のみで作業し、PRヘッドが途中で更新された場合は中止。force push禁止。
- 元のGit treeと `research/procon-2026/` のGit tree SHAが一致することを確認。メタデータとチーム側ルート `Cargo.toml` の不変、Subtree外の差分がないことを確認。
- GitHub Actionsでは取り込んだコードを実行しない。コンパイル/テスト/CIは別途行う。
- PRを自動マージしない。

## 継続同期

初回マージ後、必要に応じてチーム側に新しい同期ブランチ・PRを作成する。現状のworkflowは**PR #26専用**であり、初回完了後は通常の新規チームPRに対応するよう同期先ブランチを設定し直す必要がある。同期スクリプト自体はチーム側の `scripts/research-subtree.sh pull` でも利用可能。

## 権限・失敗時

- `Resource not accessible by integration`、`Repository not found`：チーム側トークンのスコープ/Org承認/招待を確認。
- `Missing secret PROCON_TEAM_WRITE_TOKEN`：個人側Actions Secretが未設定。
- 個人側 `main` のSHA不一致：共有ファイルと最新コミットを再点検。
- PR head競合：Pushせず停止する。チーム側の最新PRに取り直す。
- `git subtree`/Git tree不一致：Pushせず停止する。同期方式をファイルコピーへ変更しない。

※この手順はワークフローの設計であり、実行した・マージした証明ではない。Actions runとチーム側PR差分で確認する。
