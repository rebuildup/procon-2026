# Release

Release sprint と version 管理の運用ルール。

## Version 規約

- 1 sprint = 1 target semantic version。
- 開始時に `main` から `release-<major>-<minor>-<patch>` を切る。
- patch: バグ修正のみ
- minor: 後方互換 feature 追加
- major: breaking change を含む

branch 名フォーマット:

```text
release-<major>-<minor>-<patch>
```

例: `release-0-2-0`。

## Ticket flow

```
Issue (#123) → ticket branch (123) → Draft PR (123 → release-0-2-0)
                                          │
                                          ▼
                                   Ready PR (CI green, review 解決)
                                          │
                                          ▼
                                   release-0-2-0 へ merge → Issue Close
                                          │
                                          ▼
                                release-0-2-0 → main (release PR)
                                          │
                                          ▼
                                     main = v0.2.0 released
```

## Draft → Ready 条件

- acceptance criteria が実装済み
- ticket integration gate (CI 全 pass) 成功
- blocking issue 解消 / scope 外明示
- PR description が現状と一致
- release branch との conflict / staleness 解消
- latest durable checkpoint と branch state が矛盾しない

## Ticket Done 条件

- required CI green
- blocking review 解決
- release branch へ merge 済み
- Issue closed
- GitHub Projects の status が `Done`

## Release integration

sprint 対象 ticket をすべて merge した後、`just release-check release-0-2-0` を実行する。
この gate は `just ci`・protocol consistency・workspace/lock version・optional GPU・
bundle 作成と CLI version smoke を検証する。PR の green な branch 名検査だけで代替しない。

release PR:

```text
release-x-y-z → main
```

release PR テンプレート (日本語) に含めるもの:

- release goal
- included Issues / PRs
- breaking changes
- migration notes
- full validation result (CI artifact リンク)
- known limitations
- version / release metadata

merge 後 `main` がその version の released state。

## Hotfix / patch release

`main` から直接 `release-x-y-z+1` を切る。後方互換 fix のみ。ticket branch は同じ規則 (`hotfix-issue-number`)。

## Tag / release notes

- tag は release PR merge 直後に `git tag -s vX.Y.Z -m "vX.Y.Z"` でローカル署名する。
- release-drafter (`/.github/release-drafter.yml`) がドラフトを生成する。
- CHANGELOG エントリは release PR body に集約する (個別 crate ごとの CHANGELOG は当面なし)。

## Failure mode

- release PR で CI が赤になった場合: 該当 PR だけ revert して再度 release gate を回す。
- main が壊れた場合: 直前の release branch から hotfix を立てる。
- 同期が崩れた場合: `git fetch origin && git rebase origin/main` で整列し、release PR を re-open。