# Solver Exploration Workflow

solver の性能探索では、通常の Issue / branch / PR invariant を維持したまま、候補同士の依存関係を stacked PR として表現する。

この文書は solver candidate の Git / Issue / PR 上の canonical delivery rule である。

## 目的

解法候補を一本の branch 上で上書きし続けず、各 candidate を独立した durable state として保持する。

探索構造は線形 stack に限定せず、親 candidate から複数の sibling を派生できる DAG として扱う。

```text
release-x-y-z
└─ A
   ├─ A1
   │  └─ A1a
   └─ A2
```

各 node は最低限、Issue、branch、PR、candidate ID、source commit SHA を持つ。

## Invariants

### 1 candidate delta = 1 Issue = 1 branch = 1 PR

通常の ticket invariant を崩さない。

- 1つの意味のある candidate delta ごとに Issue を作る。
- branch 名は Issue 番号のみ。
- meaningful first commit の後に Draft PR を作る。
- candidate を既存 branch に上書きして比較可能性を失わせない。

### Root candidate

新しい探索系列の root は active release branch の最新 validated state から切る。

```text
branch 201 from release-0-2-0
PR: 201 -> release-0-2-0
```

### Child candidate

親 candidate を前提にした改良は、親 branch から切る。

```text
branch 202 from 201
PR: 202 -> 201
```

release branch から親の変更を再実装してはならない。

### Sibling candidate

同じ親から独立した複数案を試す場合、それぞれ child Issue / branch / PR を作る。

```text
201
├─ 202 -> 201
├─ 203 -> 201
└─ 204 -> 201
```

sibling は可能な限り同一 evaluator version、corpus、seed set で比較する。

## Candidate identity

branch 名は delivery identity であり、solver identity ではない。

評価対象には stable candidate ID を与える。

例:

```text
loop:initial=greedy:iterations=32
loop:initial=random:seed=42:iterations=64
loop:initial=random:seed=42:iterations=64:2opt=true
```

評価 artifact には最低限、candidate ID と source commit SHA の両方を保存する。

同じ candidate ID でも source SHA が変われば別の evaluated state として扱う。

## PR contract

stacked PR の review scope は親 candidate に対する delta のみ。

PR 本文には最低限以下を記載する。

- candidate ID
- parent candidate ID
- parent Issue / PR
- parent source SHA
- hypothesis
- implementation delta
- expected mechanism
- evaluation command
- evaluation status
- #178 offline evaluation evidence
- #179 arena evidence（存在する場合のみ）
- known descendants（存在する場合）

親から継承した変更を child PR の新規成果として説明しない。

## Evidence integrity

評価結果、review、arena result が commit SHA を evidence として参照した後、その evidence を history rewrite で別内容に読み替えてはならない。

特に descendants を持つ published candidate branch では、force-push / destructive rebase を原則禁止する。

親の追加変更を子へ伝播した結果 source SHA が変わる場合、旧評価は historical evidence として保持し、新 SHA を再評価する。

「同名 candidate だから以前の結果を current proof として使える」と判断してはならない。

## Evaluation

canonical offline performance evaluation は #178 の evaluator を使う。

比較可能な candidate は、同じ条件で評価する。

- evaluator version
- fixture corpus
- generated-case seed set
- scoring semantics
- timeout policy

条件が異なる結果を直接 winner evidence として扱わない。

性能が悪い candidate も有効な探索結果である。Issue / PR / commit / evaluation evidence を残し、同じ仮説を無意味に再探索しない。

#179 の外部 arena は selected candidate の追加 evidence として使う。大量 parameter sweep を arena へ流さない。

## Promotion

### Parent accepted

親 PR が active release branch に merge されたら、direct child PR の base を親 branch から active release branch へ retarget する。

retarget 後に diff が child delta のみであることを確認する。

さらに深い descendant は、直近の未統合 parent を base とし続ける。

例:

```text
before:
release <- 201 <- 202 <- 204

201 merged:

release <- 202 <- 204
```

### Parent rejected

親 candidate が rejected / closed された場合、child をそのまま release branch へ retarget してはならない。

child に独立した価値がある場合:

1. 採用可能な ancestor を決める。
2. 必要な child delta をその ancestor 上へ移植する。
3. 新しい source SHA を作る。
4. canonical evaluator で再評価する。
5. PR base / parent metadata / evidence を更新する。

棄却 ancestor に依存した評価結果を current evidence として再利用しない。

## Merge order

採用する lineage は ancestor から順に merge する。

```text
release
  <- parent
  <- child
  <- grandchild
```

descendant を ancestor より先に active release branch へ統合しない。

merge method は通常の project policy に従う。solver exploration のために別の merge mode を導入しない。

## Branch lifetime

descendant PR が存在する candidate branch は原則削除しない。

削除できるのは以下のいずれか。

- descendants がすべて merge / close 済み
- descendants が別 ancestor へ移植済み
- durable evidence が commit SHA で追跡可能で branch ref 自体が不要

## Agent behavior

AI agent は solver 改良案を見つけても、既存 candidate を自動的に置換しない。

独立比較できる変更なら:

1. current candidate と evidence を確認する。
2. child Issue を作成する。
3. parent branch から Issue 番号 branch を作成する。
4. parent branch 向け Draft PR を作成する。
5. candidate ID を評価可能な形で登録する。
6. #178 で parent / sibling と比較する。
7. 有望ならその candidate からさらに child を派生する。
8. 不採用なら理由と evidence を残して close する。

複数案が独立なら sibling branches として並列に進めてよい。

## Relationship to evaluation systems

| System | Role |
|---|---|
| #178 | canonical offline evaluator。大量 candidate × corpus の高速・決定的比較 |
| #179 | selected candidate の外部 arena 検証 |
| This workflow | candidate を Git / Issue / PR 上で派生・比較・昇格する delivery model |

Git graph と evaluator の candidate graph は対応させるが、branch 名だけを性能評価 identity として使用しない。
