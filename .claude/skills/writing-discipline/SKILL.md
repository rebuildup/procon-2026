---
name: writing-discipline
description: 仕様書・Issue・PR本文・ADR・comment・checkpointを、reader-oriented proseへ整えるSelect -> Compose -> Reread pipelineを適用する時に使用する。
---

# Writing Discipline

書き始めない。書き終えてから出す。

**Select -> Compose -> Reread** pipelineで、文章を独立したreader-facing artifactへ整える。

## When to use

次の場合に使用する。

- Issue / PR本文 / Review comment / Discussion replyを書く
- ADR / design doc / architecture docを書く
- spec / onboarding / recovery docを書く
- checkpoint / handoff messageを書く
- documentation更新を書く
- 仕様をreader-facing proseへ整える

次だけを理由に起動しない。

- commit message (短文、format固定)
- inline code comment (局所説明)
- 1行の短文acknowledge
- 機械生成log / artifact

## Core principle

> **writing is for the reader, not for the writer.**

AI agentは自分のthinking / 試行過程 / 内部contextを保存したい誘惑があるが、それはreader-facing proseではない。

readerは:

- 必要なdecision / actionを取り出せる
- 曖昧さ0で解釈できる
- 必要なevidence付きで判断できる
- 単独documentとして理解できる

状態で読める文章を書く。

## Pipeline

### 1. Select

書く前に、何を言うかを絞る。

最低限:

- target reader
- readerが必要とするdecision / signal
- readerが既知のcontext
- readerが知らないcontext
- 必要最小限のinformation unit

「全部書かないと伝わらない」前提を置かない。
「これだけ読めば次の一手が取れる」まで絞る。

### 2. Compose

draftを書く。

最低限のrule:

- 1 paragraph = 1 idea
- 1 sentence = 1 message
- lead文で要点
- evidence / data / 具体的pointを含める
- chronological narrativeを書かない (state / structureで書く)
- decoration / emoji / hedgingでsignal性を薄めない
- 1 message内で複数のdecision依頼を混在させない
- 過去chat / private memory / 中間trial-and-errorを書かない

### 3. Reread

書いた後、reader視点でreviewする。

最低1回、次の観点でrereadする:

- 1 message単位で理解できるか
- decision pointが明確か
- 必要なevidenceが揃っているか
- ambiguous / vagueな語が残っていないか
- 古いcontext参照 / 過去chat referenceが残っていないか
- chronology dump / narrativeが混じっていないか
- readerが必要としない情報が残っていないか
- decoration / emoji / hedgingが混じっていないか
- acceptance criteria / next action / blocker / decisionが明示されているか

rereadで問題があれば戻って書き直す。

## Reader-oriented prose checklist

最低限:

- target readerが何かを書頭に置く
- lead文で要点を提示
- 必要なcontextは最小限
- evidence / data / 具体的pointでsupport
- decision / action / blockerを明示
- 曖昧な言語化を排除
- 推測 / 仮説をfactsと区別
- 既知のcontextを再説明しない
- 同じdocument内で同じfieldを二重管理しない
- reader-oriented terminologyを使用
- chronologyよりstructureで書く

## Document type guidance

### Issue / PR

- 日本語
- 目的 / scope / acceptance criteria / validation evidence / 関連dependency
- reviewerが必要とするdecision point
- PR metadata (linked Issue / assignee / reviewer / labels / target release / stack context) はGitHub native surfaceで管理、本文と二重管理しない

### ADR

- Context / Decision / Status / Consequences
- 該当decisionのbackgroundと影響範囲
- superseded / superseding関係を明示
- bidirectional reference

### Architecture / design doc

- major components / dependency direction / data flow
- visual aids (Mermaid / diagram) を必要に応じて
- decision historyはADRへ分離

### Onboarding / recovery doc

- 会話履歴なしで理解可能
- canonical command / path / entry pointを明示
- recovery pathをfresh agentで再現可能

### Checkpoint / handoff

- durable state pointer (branch / commit / Issue / PR)
- next action
- blocker
- 必要なskill / gate
- last validation evidence

## Anti-patterns

禁止:

- chronology dump (何をしたかを時系列で書く)
- context dump (chat履歴 / 内部monologueを書く)
- narrative monologue (AI自身の「考えた」「迷った」を書く)
- decoration / emoji / hedgingで埋める
- vague / ambiguous な言語化
- factsとspeculationを区別しない
- 既知contextを再説明する
- 「後で詳細を書く」「いずれ述べる」placeholder
- readerが必要としない情報の詰め込み
- 同じfieldをprose / metadata / commentに二重管理
- 1 messageに複数decision依頼を混在
- 過去chat / session / private memoryへのreferenceのみでreaderが辿れない
- 「美しい / かっこいい / 好みの」軸のみでの選択
- 過去version / past stateへの参照を書留める
- 「要約すると」「まとめると」等のnarrative filler

## Quality bar

書いた文章は次のstateで出す。

- readerが1 message / 1 document / 1 PR単位で理解できる
- decision / action / blockerがreader側で即座に取れる
- 必要なevidenceが揃っている
- ambiguous / vagueな箇所がない
- decoration / emoji / hedgingがない
- 過去chat参照だけで理解できない箇所がない
- reader-oriented proseとして整っている

## Integration with other Skills

### design-refinement

仕様 / design docは本Skillに従いSelect -> Compose -> Rereadで整える。reader-oriented proseとして渡す。

### github-delivery

Issue / PR本文 / Review commentは本Skillに従う。mutable state (CI / review status / ahead-behind) はGitHub native surfaceでcanonicalに管理し、proseへは意味がある時だけ書く。

### onboarding

onboarding docは本Skillに従う。conversation / task contextなしで文章単体を理解可能にする。

### agent-recovery

checkpoint / handoff messageは本Skillに従う。durable state pointer + next action + blocker + evidenceをreader-oriented proseとして残す。

### interaction-discipline

active work messageは本Skillと interaction-disciplineに従う。両者は整合する。

## Goal

**書いた文章を、readerが単独documentとして読み、decision / actionを即座に取れるstateに整える。chronology dump / context dump / narrative monologueを書かず、Select -> Compose -> Rereadで独立したreader-facing artifactへ磨き込む。**