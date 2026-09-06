# ADR-0003: 1 sprint = 1 target semantic version、`release-x-y-z` branch を active line にする

- Status: Accepted
- Date: 2026-09-07
- Deciders: rebuildup

## Context

複数 AI エージェント並行駆動 + 短期決戦のコンテキストで、開発進捗の integration line をどう切るか。

選択肢:

| 案 | 概要 | 利点 | 欠点 |
|---|---|---|---|
| A | trunk-based (常に main に merge) | 単純 | race / partial feature の merge 制御が困難 |
| B | GitHub Flow (feature branch + PR) | 一般的 | sprint と version の紐付けが弱 |
| C | release branch sprint | target version と integration line が一致 | branch 数が増える |

## Decision

**C を採用**。

- sprint 開始時に `main` から `release-<major>-<minor>-<patch>` を切る。
- 各 Issue は `123` のような番号のみの ticket branch を切る。
- ticket branch から `release-x-y-z` への Draft PR を早期作成。
- release PR (`release-x-y-z` → `main`) で統合。

## Consequences

### Positive

- sprint と version が 1:1 対応し、進捗が GitHub Projects / branch / Issue すべてで同期される。
- release 取り込み時に version bump と changelog を 1 PR に集約できる。
- 受験期間中に release を巻き戻し / hotfix しやすい。

### Negative

- branch 数が増える (release branch × N + ticket branch × M)。
- stale release branch を stale 検知する仕組みが必要。

### Mitigation

- `release-x-y-z` branch は sprint ごとに 1 本だけ active に保つ。
- CI の `concurrency` で同一 branch への重複 push を制御。
- Project board の WIP limit で release branch への未統合 PR 数を制限。

## Follow-ups

- ADR-0004 以降で release drafter / dependabot と組み合わせて運用。