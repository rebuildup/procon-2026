# チームリポジトリへのGit Subtree同期（GitHub Actions）

## 目的

個人研究 `rebuildup/procon-2026` の `main` を、チームの非公開 `cslab-nituc/procon2026` の既存PRブランチへ `research/procon-2026/` として投入する。初回はPR #26の `link-personal-research`、更新時は `sync-research-YYYYMMDD` を利用する。`git subtree --squash` の正規の履歴を残す。初回はadd、以降はpull。チームの `main` を直接変更しない。

Workflow: `.github/workflows/sync-research-to-team.yml`。**手動起動のみ**。個人側 `main` にこのworkflowがマージされるまで、GitHub UIのRun workflowからは起動できない。

## 認証（最初に必要）

1. 個人側の GitHub Actions `GITHUB_TOKEN` は個人リポジトリの読取に使う。別のOrganizationリポジトリには書き込めない。
2. **推奨：チーム側GitHub App** を `cslab-nituc/procon2026` のみにインストールし、Repository permissionsは `Contents: Read and write`（必要なMetadata readは自動付与）とする。組織管理者によるインストール承認が必要。
3. 個人側ActionsのRepository Variable **`PROCON_TEAM_APP_CLIENT_ID`** にAppのClient ID、Repository Secret **`PROCON_TEAM_APP_PRIVATE_KEY`** にそのAppの秘密鍵を登録する。ワークフローが `actions/create-github-app-token@v3` でインストール先を `cslab-nituc/procon2026` に限定した**短命のトークン**を発行し、ジョブ終了時に失効させる。
4. **代替：PAT**。GitHub Appを利用できない場合、個人側のRepository Secret **`PROCON_TEAM_WRITE_TOKEN`** にチーム側への書込トークンを登録する。Fine-grained PATを利用できる場合はResource owner `cslab-nituc`、対象 `procon2026`、`Contents: Read and write` に限定し、必要ならOrg管理者の承認を得る。
5. **外部CollaboratorはFine-grained PATでのアクセスに制約がある。** 対象Organizationへメンバー追加してもらうか、チーム側GitHub App方式を優先する。Classic PAT（広い`repo`権限）は最小権限にできないので非推奨。AppのClient ID変数が存在する場合はApp経路を優先し、失敗しても自動的にはPATへフォールバックしない。

Secret登録先：`https://github.com/rebuildup/procon-2026/settings/secrets/actions`（GitHub Settings → Secrets and variables → Actions）。Secret値をIssue/PR/ログや会話へ貼らない。

## 初回実行

1. PRを個人側のreleaseブランチに統合し、その後、正式なrelease PR経由で `main` へ反映する。GitHubの `workflow_dispatch` はデフォルトブランチにworkflowファイルが必要。
2. チーム側PR #26がまだDraftで、 `link-personal-research` ブランチが存在し、`research/procon-2026/` が未追加であることを確認。
3. 個人側 `main` に含まれる全追跡ファイルがチーム内に複製可能であることを確認。PDF、ZIP、動画も含まれる。機密情報や共有条件に問題がある場合は実行しない。
4. Actions → **Sync research subtree to team PR** → Run workflow。`approve_full_copy=true` を明示し、`team_branch=link-personal-research` とする。必要なら `expected_source_sha` に確認済みコミットSHAを指定し、元のmainが更新されていたら停止させる。
5. 成功時、チーム側 PR #26 に正規のSubtreeコミットが追加される。チームPRはDraftのままなので、変更をレビューして検証後にReady化・マージする。

## 実行が行う安全確認

- 実際の個人 `main` のコミットSHAを記録し、指定SHAとの一致を検証。
- 明白な秘密鍵・実 `.env` の追跡ファイル名があれば停止（**ファイル内容の機密監査の代わりにはならない**）。
- チーム側 `link-personal-research` のみで作業し、PRヘッドが途中で更新された場合は中止。force push禁止。
- 元のGit treeと `research/procon-2026/` のGit tree SHAが一致することを確認。メタデータとチーム側ルート `Cargo.toml` の不変、Subtree外の差分がないことを確認。
- GitHub Actionsでは取り込んだコードを実行しない。コンパイル/テスト/CIは別途行う。
- PRを自動マージしない。

## 継続同期

初回PRのマージ後、チーム側 `main` の最新状態から `sync-research-YYYYMMDD` というブランチを作成してGitHubへPushする。個人側Actionsで `team_branch=sync-research-YYYYMMDD` を指定して実行すると、Git Subtreeの `pull --squash` がそのブランチに適用される。結果をチーム側で新規PRとして作成し、レビューとCIを確認してからマージする。workflowは `link-personal-research` または `sync-research-YYYYMMDD`（任意で短い接尾辞）以外にはPushしない。

同期スクリプトはチーム側の `scripts/research-subtree.sh pull` でも利用できる。

## 権限・失敗時

- `Resource not accessible by integration`、`Repository not found`：チーム側トークンのスコープ/Org承認/招待を確認。
- 認証チェック失敗：GitHub AppのClient ID/秘密鍵、インストール先権限、またはPATのActions Secretを確認。
- 個人側 `main` のSHA不一致：共有ファイルと最新コミットを再点検。
- PR head競合：Pushせず停止する。チーム側の最新PRに取り直す。
- `git subtree`/Git tree不一致：Pushせず停止する。同期方式をファイルコピーへ変更しない。

※この手順はワークフローの設計であり、実行した・マージした証明ではない。Actions runとチーム側PR差分で確認する。
