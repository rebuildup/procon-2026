---
name: policy-evaluation
description: Agent / organization / systemの「policy」を、deterministic testまたはlatent evalのどちらで評価すべきかを判断し、設計・実装・運用する時に使用する。
---

# Policy Evaluation

policyを「とりあえずLLMに判定させる」で運用しない。policyの性質によって **deterministic evaluation** と **latent evaluation** を使い分ける。

## When to use

次の場合に使用する。

- coding agent / review agent / moderation agent / safety policy / business ruleの評価methodを設計する
- 既存evaluationが flaky / over-permissive / over-strict / brittleかを診断する
- policy / guideline / ruleを実装可能な判定器へ落とす
- eval datasetのlabeling ruleを定義する
- human reviewをどこまでpolicy評価に組み込むか設計する
- new policy変更が既存policyへ与える影響を評価する

次だけを理由に起動しない。

- 単純なunit test / integration testで十分
- 1caseだけのspot check
- subjective preferenceだけ

## Core principle

**policyの「何を正しいと呼ぶか」を具体的に書けるなら、まずdeterministic evaluation。書けない、または言語化不能なqualityならlatent evaluation。**

## 1. Decompose the policy

policyを性質ごとに分解する。

最低限:

- 構造的rule (schema / format / boundary / signature / API contract)
- 論理rule (禁止pattern / 必須pattern / ordering / arithmetic)
- 参照rule (naming / citation / reference consistency)
- semantic rule (intent / user-facing / business rule / 推奨度)
- judgment rule (美学 / 妥当性 / 適切性 / 説明力)

各ruleについて:

- 判定基準を **predicateとして書けるか**
- 判定基準を **oracleとして実行できるか**
- 判定基準が **観測可能か**
- 評価unitが **finite / enumerableか**

を書けるならdeterministic。
書けない / 言語化不能 / 観測不能ならlatent。

## 2. Deterministic vs latent

### Deterministic evaluation

- predicate / oracle / grammar / regex / schema / linter / type-check / property testで判定
- 同じinputに対して常に同じresult
- failureを具体的pointで示せる
- CI / pre-merge / lintで高速に回せる
- 評価coverageを明示できる

主な対象:

- structure / format / naming / schema
- API contract / boundary signature
- 禁止pattern / 必須pattern
- ordering / arithmetic / state machine
- reference consistency (link / citation / import)
- security rule (secret / permission / vulnerability class)
- lint / type / compile-time property

### Latent evaluation

- LLM / 人間 / 統計的metricで判定
- probabilistic / noisy / 揺らぐ
- coverageが有限sample
- 人間 / model update / prompt差で結果が変わる
- 主にsemantic / judgment / aesthetic / user intent対象

主な対象:

- user intent / helpfulness / 妥当性
- 説明力 / 読みやすさ / 美学
- 推薦品質 / 適切性
- ambiguous ruleの妥当性
- policy intentと実例の一致

**latent evaluationはdeterministic evaluationの代替ではなく、deterministicで書けないqualityに対する補完手段である。**

## 3. Hybrid evaluation

1つのpolicyを性質ごとに分解し、決定可能な部分をdeterministicで固め、残ったsemantic / judgment部分をlatentで評価する。

例:

```text
policy:
  structure: deterministic
  naming: deterministic
  forbidden pattern: deterministic
  reference consistency: deterministic
  semantic correctness: latent
  readability / aesthetic: latent
  business rule appropriateness: latent (with rubric)
```

**deterministicで書けるruleをlatentに投げない。latent evalのcost / noise / coverage限界を理由にdeterministic ruleを逃げない。**

## 4. Deterministic evaluation design

predicateとして書く。

最低限:

- input / output contract
- 判定predicate (pass / fail / warning)
- 失敗時の具体的point (file / line / span / rule id)
- 評価unit (1行 / 1関数 / 1file / 1commit / 1PR)
- 評価coverage (全commit / PR / 特定trigger時のみ)
- CI / pre-commit / lint / type / buildへの組み込み

既存toolchain / linter / formatter / type-checkerを優先し、車輪の再発明を避ける。

## 5. Latent evaluation design

latent evalを使う場合:

- 何をlatentに評価するかを限定する
- 評価unit (回答 / PR / commit / 提案) を定義する
- rubric / label schema / grading scaleを固定する
- datasetのlabeling ruleを定義する
- prompt / model / judge versionを記録する
- 人間review / spot check / calibration sampleを残す
- coverageとnoise levelを明示する
- deterministicで書けるruleをlatent化していないか再評価する

LLM-as-judgeを使う場合:

- judge model / version / promptを固定する
- rubricの曖昧さを最小化する
- 人間reviewとcalibrationを定期的に行う
- resultはsingle-shot判定ではなく、sample coverage + spot check + aggregate signalとして扱う
- judge modelが対象policy実装を内包しているbiasに注意する

## 6. Dataset / labeling

latent evalでは評価datasetの品質が結果の上限を決める。

最低限:

- labeling rule
- label schema
- inter-annotator agreement (人手labeling時)
- calibration sample
- 代表的edge case
- drift検出
- policy変更時の再labeling policy

labeling ruleは曖昧な言語化を排除し、predicateへ落とす。

## 7. Evaluation trigger

deterministic eval:

- pre-commit / pre-merge / CI / scheduled / runtime
- 評価unitとCI resource costからtrigger frequencyを決定

latent eval:

- periodic / sampled / spot check
- release前 / policy変更時 / 重大incident時
- 人間reviewと同時

**latent evalを毎PRでfull-coverage実行する設計は、noiseとcostを理由に評価品質を劣化させる。**

## 8. Coverage policy

deterministic eval: 評価unitを明示し、coverageを明示する (全PR / 全commit / sample / critical pathのみ)。

latent eval: sample size / sampling strategy / drift検出基準を明示する。

## 9. Anti-patterns

禁止:

- deterministicに書けるruleをLLM-judgeに投げる
- latent evalをCIの必須gateとしてfull-coverageで毎PR実行する
- judge model / prompt / rubricを未固定のまま運用する
- labeling ruleを曖昧な言語化にしておく
- 人間reviewをpolicy評価の代わりに使い続ける
- latent evalのnoiseをsingle-pass判定として扱う
- 評価unitを定義せずにscoreを集計する
- policy変更後の再labeling / re-evaluationを行わない
- policy評価の「pass」を運用rule (release / merge / approve) の唯一のsourceにする
- judge modelが対象実装のbias / self-evaluationに依存する
- latent eval失敗を「LLMのせい」で片付ける
- deterministic evalがpassなのにpolicy違反が残る状態を許容する

## 10. Integration with other Skills

### quality-gate

policy評価はquality gateのcomponent。deterministic評価はgateに統合し、latent評価はrelease gate / spot checkに統合する。

### correctness-assurance

policy評価はcorrectness保証の一部。predicateとして書けるruleはdeterministicに保証し、残ったsemantic / judgment部分をlatentで評価する。

### parallel-orchestration

latent evalは多くの場合serial / sampled / 人間review依存であり、deterministic evalとtrigger frequencyを分離する。

### agent-recovery

policy変更 / eval結果driftはcheckpointへ記録する。re-evaluation policyも明記する。

## Goal

**policyの性質をpredicateとして書けるか観測可能かによって、deterministicとlatentを明確に使い分け、deterministicで書けるruleをlatentに逃げず、書けないruleにlatent evalを限定的に使い、両者をquality gate / release gateに統合する。**