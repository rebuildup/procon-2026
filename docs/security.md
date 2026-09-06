# Security

framework / runtime / dependency の advisory を継続的に追跡し、project への実影響を判定して release へ反映する運用。

## Source priority

1. official framework / runtime / SDK security advisory
2. official release / security announcement
3. ecosystem official advisory source
4. GitHub Security Advisories / dependency alerts
5. maintainer patch information
6. trusted secondary source

## Priority 判断基準

severity だけでなく以下を総合判定する:

- exploitability
- project reachability (該当 crate / path で実際に使われているか)
- external exposure (network / filesystem / IPC 露出)
- required privilege
- impact
- fix availability
- workaround quality
- regression risk
- release timing

## Workflow

1. advisory を発見したら `.claude/skills/security-maintenance/SKILL.md` のフローに従う。
2. project に到達する (実コード path で使用されている) なら GitHub Issue 化:
   - タイトル: `[security] <CVE / advisory>: <short>`
   - labels: `type: bug`, `priority: P0..P2`, `area: <crate>`
   - target release を割り当てる
3. critical で exposed なら current sprint を中断して patch release を優先。
4. fix PR を作成し、dependency lockfile 更新と minimal change で merge。
5. release notes に advisory ID と impact を残す。

## Tooling

- **Dependabot** (`/.github/dependabot.yml`) — weekly 更新で PR を自動生成。
- **cargo audit** — 必要に応じて `cargo install cargo-audit && cargo audit`。
- **bun audit** — `bun audit` (将来追加)。

## Secret handling

- `.env` / `.env.development` / `.env.production` は git ignore。
- 代わりに `.env.example` / `.env.development.example` / `.env.production.example` を committed に置く。
- secret を commit / log / agent result / snapshot / checkpoint に含めない。
- agent が secret を見つけた場合、即座に redact し Issue で報告。

## Supply chain

- GitHub Actions は SHA pin または公式 major tag (`@v4`) を使い、未trusted PR では `pull_request_target` を避ける。
- npm / cargo dependency は weekly Dependabot で更新し、release 前に `cargo tree` / `bun pm ls` を sanity check。
- 公式 action 以外を使う場合は owner / repo / commit を明示。

## Reporting a vulnerability

セキュリティ問題を public Issue で報告せず、maintainer に直接連絡する。
当面、窓口は GitHub の private vulnerability report 機能を使う。