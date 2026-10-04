---
name: linear-release-control
description: GitHub Issuesをcanonical ticket SoTとしつつ、Linearをrelease-level planning / health / portfolio control planeとして使うprojectで、両者の責務境界・同期規則・failure時の優先順位を運用する時に使用する。
---

# Linear Release Control

このSkillは、Linearを **release-level planning / health / portfolio control plane** として使うprojectのみを対象とする。

Linearを全面GitHub mirrorにしたり、ticket-level SoTをLinearに置いたりすることは対象ではない。canonical ticket SoTはGitHub Issuesに置く。

## When to use

次の場合に使用する。

- release / sprint / initiative / portfolio health / roadmapをLinearで可視化する
- GitHub IssueとLinear Project / Initiative / Cycleの同期を行う
- Linear -> GitHub、または GitHub -> Linearの反映を制御する
- Linear API / Webhook / automationをGitHub workflowと組み合わせる
- Linear profile採用projectで、release完了 / carry-over / failure handlingをreconcileする

次だけを理由に起動しない。

- GitHub Projectsのみで十分
- Linearを全面mirrorしているproject (そのprojectのcanonical ruleに従う)
- release-level planning / portfolio healthが不要な個人規模project

## Boundary

| Concern | Canonical SoT | Role |
| --- | --- | --- |
| Ticket definition (Issue, PR, dependency) | GitHub | Source |
| Ticket lifecycle (open / review / merge / close) | GitHub | Source |
| Release branch / tag / release PR | GitHub | Source |
| Sprint / Cycle / Initiative planning summary | Linear | Projection |
| Release-level health / portfolio state | Linear | Projection |
| Cross-release trend / capacity / carry-over analysis | Linear | Projection |

**GitHub evidenceをLinearへprojectionする。Linearをticket SoTに昇格させない。**

## Don't do

禁止:

- GitHub IssueをLinear Issueへ1:1完全mirrorする
- Linear側でticket definition / dependency graph / acceptance criteriaを再入力する
- Linearをcanonical SoTとして扱い、GitHubをshadow扱いする
- Linear -> GitHubの自動pushでGitHub Issueのdependencyやacceptance criteriaを上書きする
- GitHub -> Linearのauto mirrorで、release planning以外の情報をLinearへ漏らす
- GitHub Projects / Linearを併用して同じfieldを両者でcanonicalに二重管理する
- Linearで定義したticket metadataを、GitHubで唯一のcanonical sourceとして扱う

## Sync direction

### GitHub -> Linear (release-level projectionのみ)

同期対象:

- release branch name / version
- included Issue list (release candidate scope)
- Issue status (open / closed / draft)
- Issue labels (release-relevant labels)
- release-level milestones / Cycle association
- release date / target date (project-defined format only)

同期対象外:

- Issue description全文
- acceptance criteria本文
- PR review discussion本文
- dependency graphの内部細部 (Project / Initiative summary levelだけで十分)
- work-in-progress詳細 (PR / branch / check run等)

### Linear -> GitHub (なしを基本)

LinearはGitHubへ書き戻さない。

例外:

- Linear webhook / automationが、明示的にuserが指定したrelease-level signal (release health / Cycle update) をGitHub Discussion / Release comment / Issue release summaryへproject-defined mappingで投影する場合

これは **mirrorではなくprojection** として扱い、canonical SoTはGitHub側へ置いたままにする。

## Failure handling

GitHubとLinearのstateが乖離した場合:

1. GitHub evidenceをcanonical source of truthとして扱う
2. Linear projection側のstaleを検出し、project-defined refreshで再投影する
3. Linear側で人手編集された情報をmirror扱いにせず、release planning summaryにだけ使う
4. 乖離を自動修正せず、stale marker / logで明示する
5. user / 人間 review へ escalationが必要な乖離は明示的に報告する

## Release planning flow

1. GitHub Issuesからrelease candidateを確定
2. release branch / target versionをGitHubで確定
3. Linear Project / Initiative / Cycleをrelease単位で構成
4. GitHub evidenceからrelease-level summaryをLinearへprojection
5. release期間中はGitHub側ticket lifecycleをcanonicalに保つ
6. release PR merge後にactual release stateをLinearへ投影 (Released / Cycle update)
7. carry-over ticketは次release branchへre-planし、Linear Project / Initiativeをreconcile

## Anti-patterns

禁止:

- Linear IssueとGitHub Issueを1:1で完全mirrorする
- Linear APIにGitHub Issue descriptionを再投入する
- 同期に失敗したら自動retryで黙って回復する
- 「Linearが新しいからGitHubを捨てる」的transitionを1ticket単位で行う
- Linear stateをcanonicalとして扱い、GitHub evidenceをsecondary扱いする
- release scopeをLinear側で編集し、GitHub release branchの整合を取らない
- 同じfieldをGitHub ProjectsとLinearの両方で更新する
- GitHub Issuesに存在しないticketをLinear側で作成してcanonical扱いする

## Integration with other Skills

### github-delivery

GitHub Issues / PR / release branchはgithub-deliveryに従う。Linear projectionはrelease-level summaryに限定する。

### parallel-orchestration

Linear projection更新はsupervisor / coordinator責務。worker / subagentがLinear projectionを直接編集しない。

### agent-recovery

checkpointにはGitHub Issue / PR / branch / release stateをcanonical stateとして記録する。Linear projection stateはsecondary referenceに留める。

### onboarding

fresh agent / fresh contributorは、canonical ticket SoTがGitHubであることをonboarding docから即座に発見できる必要がある。Linear projectionの位置付けはdocs/architectureまたはdocs/releaseに明示する。

### quality-gate

release gateはGitHub evidenceで判定する。Linear projectionのstateのみでrelease readinessを判定しない。

## Goal

**GitHubをcanonical ticket SoTに維持したまま、Linearをrelease-level planning / portfolio healthのためのcontrol planeとして使う。両者の責務境界を明確にし、同期failure時にもGitHub evidenceを優先して運用する。**