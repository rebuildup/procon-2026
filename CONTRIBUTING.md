# Contributing

procon-2026 への貢献方法。Issue 駆動 / Release sprint ベースのチーム開発を前提にしている。

## 基本原則

- **1 Issue = 1 ticket branch = 1 Draft PR → release PR**
- branch 名は **Issue 番号のみ** (`123`、`124`)。`issue/` 等の prefix や slug を入れない。
- **source code / commit message は英語**、**docs / Issue / PR / review は日本語**。
- `main` は released state。active development は `release-x-y-z` branch 上で行う。
- `target_version` は Issue / Project 上で明示し、対応する release branch と整合させる。
- Solver 性能探索に限り、`docs/contest/solver-exploration.md` の stacked PR rule を適用できる。この場合も **1 candidate delta = 1 Issue = 1 branch = 1 PR** は維持する。

## 開発の流れ

1. 作業を開始する前に Issue があるか確認する。なければ `feature.md` または `bug.md` テンプレートで起票する。
2. Issue に `target_release`、`acceptance criteria`、`verification policy` を必ず書く。
3. `main` (または直近の active release branch) から branch を切る:

   ```bash
   git fetch origin
   git switch -c 123                # Issue #123 の branch
   ```

4. meaningful な最初の commit を push したら、target release branch 宛に Draft PR を出す:

   ```bash
   git push -u origin 123
   gh pr create --draft \
     --base release-0-2-0 \
     --title "[#123] domain: add initial Turn validator" \
     --body-file .github/PULL_REQUEST_TEMPLATE.md
   ```

5. ローカルで quality gate を回す:

   ```bash
   just check && just test
   ```

6. CI 緑 + reviewer 承認 + acceptance criteria 達成で Draft → Ready。
7. Release PR (release-x-y-z → main) でマージし、Issue を Close。

## Solver exploration の例外

solver candidate の比較・派生では、PR の base を常に release branch に固定しない。

親 candidate を前提にする child candidate は親 branch から切り、親 branch 向け Draft PR を作る。

```text
release-0-2-0
└─ 201
   ├─ 202
   │  └─ 204
   └─ 203

PR: 201 -> release-0-2-0
PR: 202 -> 201
PR: 203 -> 201
PR: 204 -> 202
```

親が release branch に merge されたら direct child PR を release branch へ retarget する。親が rejected の場合、child は viable ancestor へ移植して再評価するまで release 候補へ昇格させない。

評価済み commit SHA は durable evidence である。descendant を持つ published candidate branch の destructive rebase / force-push は原則禁止し、SHA が変わった場合は以前の評価を current proof として再利用しない。

詳細・candidate ID・branch lifetime・agent behavior は `docs/contest/solver-exploration.md` を canonical source とする。

## Commit メッセージ

Conventional-ish 英語 prefix を使う。

```text
<scope>: <terse imperative summary>

<body: why, not what>
```

例:

```text
domain: add Turn::validate against match roster

crates/domain/src/state.rs previously allowed any author through.
This adds a roster check so that downstream crates can rely on it.
```

## Pull Request レビュー観点

- acceptance criteria がすべて満たされている
- decision precedence (`AGENTS.md` §4) と整合
- required verification level (unit / integration / contract / smoke / E2E) を満たしている
- breaking change の場合、`docs/release.md` と `crates/*/CHANGELOG` が更新されている
- secret / machine-specific path / private reasoning を含まない
- `release-x-y-z` branch の最新と rebase 済み

## AI エージェントを使う場合

`.claude/skills/` の Skill を progressive disclosure で読み、AGENTS.md の decision precedence に従う。会話履歴は SoT にせず、durable Git state を唯一の source にする。