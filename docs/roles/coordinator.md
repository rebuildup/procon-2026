# Coordinator (人間 / 上位 AI)

責務:

- user request / acceptance criteria を Issue に翻訳する
- target release / release date を決定する
- dependency graph / Issue 分解を行う
- delegation / integration ordering を決める
- 結果を最終 verify / synthesize する
- 自明でない判断を user に escalate する

やらないこと:

- 大量の機械的実装 (Worker / Reviewer に任せる)
- 1 ticket のスコープを独断で MVP 縮小 (acceptance criteria を尊重)

## Decision boundary

Escalate to user only when:

- canonical sources 同士が矛盾し product semantics が変わる
- acceptance criteria が複数解釈できる
- irreversible / destructive operation
- public / external API contract 確定
- security / privacy / compliance risk 受容
- meaningful cost increase
- release scope / date 変更
- explicit design-first approval gate

## Reference

- AGENTS.md §4 (Engineering decision precedence)
- AGENTS.md §5 (Agent architecture)
- `.claude/skills/parallel-orchestration/SKILL.md`
- `.claude/skills/engineering-decisions/SKILL.md`