# Recovery

AI agent / sandbox / context loss からの復旧方針。durable state から fresh agent が再構成できることが canonical path。

## Failure model

最低限次に備える:

- model / session context loss
- agent process crash / cancellation
- IDE / terminal restart
- parent agent crash while child continues
- child / subagent crash
- sandbox / container / VM recreation
- Supervisor restart
- transient network / provider failure
- host reboot
- context-window exhaustion

RPO / RTO は project 規模に応じて `docs/adr/` に記録する。

## Durable recovery sources (優先順)

1. GitHub Issue / Project
2. target release branch (`release-x-y-z`)
3. ticket branch / commit graph
4. Draft / Ready PR / review / CI state
5. committed design / ADR / Skills / docs
6. immutable worker / subagent results
7. structured recovery checkpoint (後述)

transient (会話 ID / agent ID / shell history / IDE state) は optimization であって SoT ではない。

## Structured recovery checkpoint

private chain-of-thought は保存しない。fresh agent が再開できる operational state だけを外部化する。

最低限のフィールド:

```text
schema_version
issue_id
target_release
ticket_branch
pr_number
base_sha
checkpoint_sha_or_snapshot
execution_generation
status
completed_steps
next_steps
pending_validation
active_children
integrated_child_results
external_side_effects
blockers
decision_refs
artifact_refs
updated_at
```

保存先候補:

- PR description (Draft PR の最新 push で更新)
- Issue comment (長い checkpoint 用)
- durable branch (`refs/notes/*` または `refs/checkpoints/*`)

## Checkpoint 戦略

- **soft checkpoint**: same host / sandbox recovery 向け。filesystem snapshot, Supervisor journal, native session state。
- **hard checkpoint**: sandbox / provider 損失に耐える境界。meaningful code / work state が durable remote infrastructure (GitHub / S3 相当) から到達可能であること。

すべての小 edit を remote commit する必要はない。RPO / task length / provider TTL から checkpoint 頻度を決める。

## Checkpoint trigger

次の場合に checkpoint を検討する:

- meaningful implementation milestone
- risky refactor / migration
- child spawn 前
- child result integration 直後
- long validation 前後
- external side effect 前後
- user / external input 待ち
- provider TTL / shutdown 接近
- graceful cancellation / shutdown signal
- context limit 接近

## Recovery algorithm (fresh agent)

1. Issue / PR / target release を特定する。
2. ticket branch / remote commit graph を fetch。
3. latest valid checkpoint を読み込む。
4. canonical policy / design / decision refs を確認する。
5. active children を Supervisor から再発見する。
6. checkpoint から workspace を recreate。
7. completed / pending validation を再評価。
8. external side effect の actual remote state を確認。
9. stale base / conflicting integration を確認。
10. remaining plan を再構成。
11. safe な最小 verification で reconstructed state を確認。
12. execution generation / lease を更新して続行。

native resume が成功しても、branch / PR / checkpoint との整合を必ず確認してから本作業に戻る。

## Parent / child recovery

- child lifecycle は Supervisor が所有する。parent model process に所有させない。
- parent 死亡時に safe なら child を即 cancel しない。
- recovered parent / coordinator は:
  - child 一覧を再発見
  - input snapshot / execution generation を確認
  - running / completed / failed / orphaned を分類
  - completed result を immutable result として回収
  - stale child result は自動統合しない
  - 必要なら retry / resume / re-spawn

network partition や timeout 後の split-brain を防ぐため、各 task に lease または generation / fencing token を持たせる。

## External side effects / idempotency

Git 外の操作は中断復旧で危険。可能なら idempotency key を使う。

例:

- production / staging deploy
- DB migration
- package publish
- release / tag creation
- cloud resource mutation
- notification / email / comment creation
- billing / cost-producing operation

side effect 前に intent、後に result / remote identifier を durable journal に記録する。
recovery 時は remote actual state を確認してから retry する。

`command returned no response = operation did not happen` と推測してはならない。