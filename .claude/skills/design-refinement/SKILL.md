---
name: design-refinement
description: 仕様・設計・要件に対し、書く前にreadし、過剰に尋ねず、最小限の明確化で実装可能なspecificationへ磨き込む時に使用する。
---

# Design Refinement

AI agentが仕様・要件・指示を受け取ったら、いきなり書き始めない。最初にreadし、最小限の確認で実装可能な形へ持ち上げる。

## Core principle

> **read relentlessly, ask minimally.**

最初に「readして理解する」ことを最優先する。読んで分かる事は再度聞かない。
本当に必要な質問だけ、1度にまとめて、答えやすい形で尋ねる。

## When to use

次の場合に使用する。

- 新規仕様 / Issue / 依頼の意図を実装可能なlevelへdetail化したい
- 指示が曖昧で複数の意味が取れる
- 既存実装majorityと新指示の整合を判断したい
- user-visible flow / data shape / boundaryを確定したい
- 既存productのextensionで既存contractへの影響範囲を特定したい
- 質問疲れを増やさず必要なdecision pointを拾いたい
- 設計specificationを後続worker / reviewer / future-selfへ伝えたい

次だけを理由に起動しない。

- 仕様が既に明確に実装可能
- 1行修正レベルのtrivialな変更
- 既に完全なspecification / ADR / Issue descriptionが存在し、不足が無い

## Pipeline

### 1. Read

仕様・指示・既存実装・周辺文書を全て読む。

最低限:

- 仕様 / Issue / 依頼本文
- 関連ADR / design doc
- 既存実装majority (in-repo canonical pattern)
- dependency / blocked-by / linked Issue
- test / validation方針
- external boundary contract

理解のunitは「文」ではなく「境界 / contract / data shape / flow」。

### 2. Identify decision points

仕様 → 実装で曖昧さが残るdecision pointだけを抜き出す。

最低限の観点:

- user-visible flow
- input / output / data shape
- persistence / state / lifecycle
- boundary (API / DB / queue / external)
- error / failure semantics
- performance / 規模前提
- security / trust
- observability
- migration / backward compatibility
- scope / non-scope

各decision pointについて:

- **resolved by read**: 既存実装 / 仕様文 / 慣習で確定
- **resolved by safe default**: reversible / localizedなため安全な既定で進める
- **must ask**: reversibleでない・contractを変える・scopeが変わる・外部影響がある

resolved by read / safe defaultは聞かない。
must askだけ、最小限、まとめて聞く。

### 3. Ask only what matters

質問する場合は:

- 1メッセージにまとめて投げる
- 選択肢を与える
- それぞれの影響を簡潔に示す
- 既定 / 推奨がある場合は最初に明示する

質問数そのものよりも「答えうる情報量」を最大化する。

反例:

- 確認済の前提を質問で戻す
- 1機能に10個の質問
- 答えられない抽象論 / philosophy question
- 「美しい / かっこいい / 好みの」軸で質問

### 4. Compose specification

明確化された事項をreader-orientedなspecificationに整える。

最低限含むべき項目:

- 目的 / user-visible outcome
- scope / non-scope
- 主要flow
- data shape / contract変更
- error / failure semantics
- acceptance criteria
- validation evidence要件
- 関連ADR / Issue / 依存

仕様は後続worker / reviewerが読んで実装可能なlevelまでdetail化する。
AI自身のchat contextに保存するためだけに残さない。

### 5. Decide before write

write / 実装前に:

- acceptance criteriaが判定可能な状態か
- data shape / contract / boundaryが固定されているか
- validation evidenceが決まっているか
- 依存 / stack / release branchが追跡可能か
- known limitationが明示されているか
- 必要なskill (quality-gate / correctness-assurance / parallel-orchestration / agent-recovery / writing-discipline) を識別したか

これらが揃ったらworker / subagentへ渡すcontractとして使える。

## Read first checklist

- 関連ADR
- 関連Issue / PR / Discussion
- target module / package / boundary file
- 既存test / validation script
- 既存実装majority (in-repo canonical pattern)
- dependency / build / CI定義
- 既存doc / README / onboarding doc
- past decision / rejected alternative

「読んでいない」を根拠に質問を増やさない。

## Ask checklist

次の場合のみ質問する。

- reversible / localizedな決定ではない
- public contract / data shapeが変わる
- 既存user / dataへ破壊的影響
- 外部boundary (API / DB / 3rd party)へのsemantic変更
- cost / resource / infra decision
- legal / compliance / privacy / security
- scope自体が変わる
- product semanticsの選択 (business rule)

次の場合は聞かない。

- 既存実装majorityから明白
- safe defaultでreversible
- 仕様文 / Issueで定義済
- ADRでdecision済
- 技術慣習で自明
- 単一file内・既存component内の局所最適化

## Compose rules

仕様を書く際は:

- reader-oriented
- acceptance criteriaを必ず含める
- contract / data shape / flowを明記
- 検証evidence要件を明示
- scope / non-scopeを明示
- known limitationを明示
- ambiguous / TBDを残さない

実装はspecificationが十分detail化された後に開始する。

## Anti-patterns

禁止:

- 仕様を読む前に書き始める
- 読んだ内容をchat contextへ保存し、成果物へ反映しない
- safe defaultで十分なのに質問する
- 1度に1個ずつ質問する
- 答えられない / 答えにくい形式で聞く
- 「好み」「美学」「将来のため」だけを理由に質問
- 仕様を書かずにworker / subagentを起動する
- 「specification」と称して曖昧なメモを書く
- acceptance criteriaを書かずに完了とする
- 既存実装majorityを読まずに新規patternを導入する
- reversibleで局所的な変更を質問でescalateする
- 質問後にuserから得た回答を反映せず実装を進める
- reviewerが読めないlevelで仕様を残す

## Goal

**read everything relevant, ask only what truly blocks, produce a specification a worker can implement without further clarification, and minimize user fatigue.**