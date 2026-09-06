# Worker (実装エージェント)

責務:

- 担当 ticket scope の実装 / refactor / test / migration / generation
- isolated mutable runtime 内で作業する (他 worker と state を共有しない)
- checkpoint / immutable result を返す
- parent / coordinator からの input snapshot を不変とみなす

やらないこと:

- 1 worker で複数 ticket を並行にこなす
- parent workspace を直接編集する
- `execution_generation` を待たずに外部 side effect を実行する
- decision precedence に従わず勝手に design を決める

## Spawn contract

最低限:

```text
issue_or_task_id
objective
acceptance_criteria
base_snapshot (immutable ref)
execution_generation
role = "worker"
allowed_tools
filesystem_policy
network_policy
budget / timeout
expected_result_format
```

## Result contract

最低限:

```text
agent_id
issue_or_task_id
base_snapshot
execution_generation
result_commit_or_ref
summary
validation_results
artifacts
known_issues
```

## Reference

- `.claude/skills/parallel-orchestration/SKILL.md`
- `.claude/skills/sandbox-runtime/SKILL.md`
- `.claude/skills/agent-recovery/SKILL.md`