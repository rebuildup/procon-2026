---
name: interaction-discipline
description: AI agent / 人間 / 別system間のactive work messageを、最小情報量で、最大signalで、誤読なく運ぶために設計する時に使用する。
---

# Interaction Discipline

AI agent / 人間 / 別system間のactive work messageは「自分が今考えている事をそのまま書く」のではなく、readerが必要なsignalを取り出せる形に整える。

## When to use

次の場合に使用する。

- supervisor / coordinator / workerへhandoffするmessageを書く
- userへ進捗 / blocker / decision依頼を送る
- subagentへtask briefを投げる
- PR / Issue / Discussionにcommentを書く
- checkpoint / recovery artifactへ現在のstateをexternalizeする
- external service / automationへtrigger / commandを送る
- long-running sessionで次session / next agentへstateを渡す
- AI自身のthinking monologueを書留めず、必要なmessageを整える

次だけを理由に起動しない。

- 1行のshort acknowledgment
- test / lint出力の機械的pass-through
- 純粋なthinking monologue
- 既にwriter disciplineで整ったmessage

## Core principles

### 1. Signal over volume

最小限の語数で、readerが必要なdecision / signalを取り出せる形にする。

### 2. Reader-oriented

readerが既知のcontextを持つ前提で書く。

- supervisor / coordinator: 全体delivery状態 / blocker / decision点
- worker: contract / scope / gate / acceptance
- user: decision依頼 / blocker / next action
- reviewer: change理由 / contract / validation evidence
- next session: durable state / resume point

### 3. No context dump

readerが必要としない情報を載せない。

- chat履歴のtranscript
- AI内部monologue
- intermediate trial-and-error log
- 既にGitHub / branch / Issue / artifactにある情報
- 推測 / 仮説を事実として書く
- 「私」「私達」等のself-referentialなnarration
- AI自身の「考えた」「迷った」record

### 4. State, not narrative

「何をしたか」ではなく「何が成立しているか」「何が未成立か」「何が必要か」を書く。

### 5. Decision-grade

readerが次のdecision / actionを遅滞なく取れるだけのevidenceとdecision pointを含める。

## Message categories

### Handoff to supervisor / coordinator

最低限:

- ticket / Issue ID
- current state (deliverable / blocker / retry)
- 必要なauthorization scope (PR / release / stack / bounded set)
- immutable handoff artifact pointer (commit / branch / snapshot / result)
- next action
- 必要なgate evidence

### Task brief to worker / subagent

最低限:

- scope / non-scope
- acceptance criteria
- contract / data shape / boundary変更
- validation evidence要件
- dependency / blocked-by
- 必要なskill
- escalation point

### Progress / status to user

最低限:

- 何を達成したか
- 何が未達成 / blocked か
- 何が必要か (decision / input / authorization)
- next action

### PR / Issue / Review comment

最低限:

- decision point
- 必要なevidence
- 制約 / context
- 推奨action

### Checkpoint for recovery

最低限:

- current durable state pointer
- active ticket / branch / Issue
- stack predecessor / pinned SHA
- next action
- blocker
- 必要なskill / gate
- last validation evidence

## Anti-patterns

禁止:

- 「考えてみます」「もう少し検討します」等のnarration
- intermediate trial-and-errorをtranscriptとして書く
- AI自身の内部monologueを書留める
- chat履歴を要約せず貼る
- readerが既知のcontextを再説明する
- ambiguous / vagueなstate記述
- decision pointを書かずに「help me」と書く
- 必要なevidenceなしで「完了」と書く
- 複数のmessageに分割すべき内容を1つのwall of textに詰める
- emoji / decorative markerでsignal性を薄める
- 「念のため」過剰な情報を載せる
- speculative / hypotheticalをfactsとして書く
- 同じmessage内でdecision依頼と進捗報告を混在させる
- userが答えられない抽象questionを投げる
- 過去session / 過去chat / private memoryへの参照のみでreaderが辿れない

## Quality bar

active work messageは、readerが次のdecision / actionを:

- 質問なしで取れる
- 曖昧さ0で解釈できる
- 必要なevidence付きで判断できる
- 1 message単位で理解できる

状態で書く。

## Integration with other Skills

### writing-discipline

messageをSelect -> Compose -> Rereadで整える。reader-oriented proseへ整える。

### parallel-orchestration

supervisor / coordinator / worker / subagent間のhandoff messageはこのSkillに従う。immutable checkpoint / result contractと整合する。

### agent-recovery

checkpoint messageはcurrent durable state + next action + blockerを明示する。sessionが失われてもdurable pathからresume可能にする。

### github-delivery

PR / Issue / Discussion commentはnative GitHub metadataと整合し、reader-oriented proseへ整える。mutable state (CI status / review status / ahead-behind) をproseへ重複しない。

### design-refinement

task briefはspecificationとして十分detail化されたものを渡す。workerが「明確化が必要」でblockされない状態にする。

## Goal

**最小情報量で、readerが次の一手を遅滞なく取れるmessageを、誤読なく、雑音なく、状態として届ける。**