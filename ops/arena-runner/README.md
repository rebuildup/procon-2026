# Arena independent runner

This package prepares a single-host Linux Runner for the unofficial Arena. It is disabled until an operator writes a complete handoff record outside the repository. It does not run during Cloud development or CI.

## Operating model

The Arena match boundary is every UTC quarter hour and its registration lobby opens three minutes earlier. The systemd timer starts at `:12`, `:27`, `:42`, and `:57` UTC. `arena_runner.py once` accepts a target only when it is inside that three-minute lobby. If the host is down or a timer invocation is delayed past the lobby, that slot is skipped. The timer uses `Persistent=false`, so systemd does not replay a missed event after restart.

Each invocation takes a non-blocking `flock` and keeps it through the match. Before starting the coordinator it atomically fsyncs a per-slot record. It runs one pinned coordinator process with `--exit-after-match`. A clean exit is accepted only when exactly one new `completed` Arena evidence record has the pinned source SHA, selected candidate, and a match ID. Any uncertain attempt leaves the slot active and blocks later registrations until an operator reconciles it. The evidence JSONL, runner state, and redacted logs live under the persistent state directory, outside Git.

This is intentionally a single-host design. Do not run two independent hosts against separate state directories. If a second host is later required, replace local `flock` and the JSON ledger with a shared transactional store and a tested lease before enabling it.

## Runtime pins

- Solver source: `13a0106aa5b1ac26dc0856bc93f731ca0e44f390` (`main` after PR #388;
  release tip `08aa550`. `crates/` content is identical to tag `v0.4.0` —
  `fleet` from PR #372 plus the lost-day coordinator fix `71a9c0f` from Issue #385)
- Primary: `fleet:budget=official:budget_ms=10000` with `--kinds-auto`
- Manual fallback: `anytime:budget_ms=5000:seed=42:budget=official` without `--kinds-auto`
- Arena: `https://procon37arena.online`

The fallback is allowlisted but is never selected automatically after a coordinator start. Change the runner config only after stopping the timer and reconciling the previous slot; a failure may have registered before it became visible.

## Build and install on an existing Linux host

The Cloud task and the connected Vercel account did not reveal an always-on host or a Vercel project for this repository. Do not treat a Codex Cloud task, a GitHub-hosted job, or this workspace's Docker executable as a durable runner. Use an already-approved Linux server with systemd and a persistent local volume. This package does not create or purchase infrastructure.

From a clean clone of this repository, run the build script as an operator with access to the host:

```sh
ops/arena-runner/build-pinned.sh /opt/procon-arena
install -o root -g root -m 0755 ops/arena-runner/arena_runner.py /opt/procon-arena/arena_runner.py
```

Create the dedicated service account and configuration directory, then install the unit files:

```sh
useradd --system --home-dir /var/lib/procon-arena --shell /usr/sbin/nologin proconarena
install -d -o root -g proconarena -m 0750 /etc/procon-arena
install -o root -g root -m 0644 ops/arena-runner/procon-arena-runner.service /etc/systemd/system/
install -o root -g root -m 0644 ops/arena-runner/procon-arena-runner.timer /etc/systemd/system/
```

Install `runner.json` as `root:proconarena` mode `0640`. Keep `ownership.json` absent until handoff; when created, make it `root:proconarena` mode `0640`. Create `/etc/procon-arena/infisical.env` as `root:root` mode `0600`, containing only the Machine Identity client settings and Infisical domain/project selector. Do not put the Arena token in this file. The systemd manager reads that file and starts a short-lived gated wrapper; the wrapper passes only Infisical variables to the Infisical CLI, and the token-bearing runner strips all `INFISICAL_*` variables before it starts `procon`.

The build script makes a persistent detached worktree at `/opt/procon-arena/source` and the pinned solver SHA, runs `cargo build --release --workspace --locked`, and records the binary SHA256, Rust/Cargo versions, and host target in `/opt/procon-arena/build.json`. It clears common Cargo/Rust profile and target overrides and verifies the required coordinator flags. Keep the generated manifest with the installed binary; the Runner refuses to start if either the source SHA or binary digest differs. The stable path also keeps runtime files addressed through Cargo's embedded `CARGO_MANIFEST_DIR` available.

Keep the timer stopped until all ownership and secret gates below are satisfied. Create `/etc/procon-arena/runner.json` with mode `0640` and owner `root:proconarena`:

```json
{
  "candidate": "fleet:budget=official:budget_ms=10000",
  "fallback_candidate": "anytime:budget_ms=5000:seed=42:budget=official"
}
```

The default ownership file does not exist. Only after the old operator explicitly confirms the match is finished and both coordinator and automatic continuation are stopped, the user confirms the transfer, #317 records the new owner, and mock/auth checks have passed, create `/etc/procon-arena/ownership.json` with mode `0640` and owner `root:proconarena`:

```json
{
  "enabled": true,
  "previous_operator_stopped": true,
  "previous_scheduler_stopped": true,
  "ownership_confirmed": true,
  "handoff_recorded_in_issue_317": true,
  "runner_validation_passed": true,
  "operator": "<responsible operator>",
  "confirmed_at": "<UTC timestamp>",
  "issue_317_url": "https://github.com/rebuildup/procon-2026/issues/317",
  "validation_ref": "<build and mock-test record>"
}
```

Do not infer any of these values from a Cloud process list. This file is an operator attestation, not a remote verification of the Mac or GitHub. If the handoff is not confirmed, leave the file absent.

## Infisical setup

The current Cloud environment reports the Infisical endpoint, project ID, and Universal Auth client fields as configured, and unrestricted HTTPS policy as enforced. It has no secret bindings, `PROCON_ARENA_TOKEN` is not injected, and an authenticated read of the existing Machine Identity has not been verified. No token was retrieved and no authenticated Arena request was made.

Before deployment, the project owner should:

1. In the existing Infisical project and `dev` environment, create a dedicated `/arena-runner` path containing only `PROCON_ARENA_TOKEN`. Do not put the Arena token in this repository, an environment file, issue, PR, terminal command, or deployment log.
2. Confirm the existing Machine Identity is read-only and scoped to that project, `dev`, and `/arena-runner`. Do not grant write access. If the current identity cannot read only this path, stop and have the Infisical owner review the narrowest available read permission before changing anything.
3. Provision the Machine Identity's client credentials to the host's protected Infisical runtime environment, not to Git. Keep that file root-readable only (`0600`) and do not print it. The Runner maps `INFISICAL_CLIENT_ID` and `INFISICAL_CLIENT_SECRET` to the CLI's Universal Auth environment variables; neither value is placed in arguments.
4. Verify access with the Infisical CLI without printing values. Install the supported CLI and use the existing identity. A safe check is to run `infisical login --method=universal-auth --save-as=arena-runner` with the two `INFISICAL_UNIVERSAL_AUTH_*` variables in the environment, then `infisical run --profile=arena-runner --projectId="$INFISICAL_PROJECT_ID" --env=dev --path=/arena-runner --include-imports=false --secret-overriding=false --silent -- python3 -c 'import os; raise SystemExit(0 if os.getenv("PROCON_ARENA_TOKEN") else 1)'`. It only returns an exit status and does not print the token. The dedicated path must contain no other secrets because `infisical run` exports all secrets available at that path.
5. The systemd entry point checks the ownership record, build, slot, and unresolved journal before invoking Infisical. It removes all Infisical client variables before launching the token-bearing Runner. The coordinator subprocess then receives only `PROCON_ARENA_TOKEN` among `PROCON_*` variables.

The unit file does not authenticate to Infisical while the handoff gate is closed or outside a valid slot. For each eligible slot it performs non-interactive Universal Auth login to a private `arena-runner` profile, then runs Infisical against the dedicated path. The Runner exits before persisting a slot if the token is absent. This currently remains a deployment prerequisite; the Cloud task did not authenticate to Infisical.

## Validation and evidence parity

Run the mock suite without an Arena token or network request:

```sh
python3 -m unittest discover -s ops/arena-runner -v
```

The suite covers lobby timing and missed slots, the ownership gate, token absence, environment minimization, one-attempt persistence, restart/uncertain state, fallback allowlisting, and log redaction. It does not prove that an old Mac process stopped or that the Arena accepted a live match.

The Cloud binary is a Linux x86-64 ELF built with rustc/cargo 1.99.0. The fixed-source build through `build-pinned.sh` at `/tmp/procon-arena-install/source` produced SHA256 `d8d8beeed5f5988851bd7164d5666cd2d53cb2e36d29430465429f0bd1d7e6e2`; an earlier build at `/workspace/procon-2026` produced `c339387b135632c813b6ee12be606ab947a929258fe78340cb911df4e512f819`. Both are Linux x86-64 builds of the same source/toolchain, and the difference is explained by the absolute `CARGO_MANIFEST_DIR` embedded by this CLI. The local official-server run used a Mac/Darwin arm64 environment and recorded `10172dc74ba13cae47ffc458967b7ebd552f99c1dc74b4d522b7f90b81feb8a2`; equality is not expected across Mach-O/ELF and arm64/x86-64 builds. The deployment build uses a stable `/opt/procon-arena/source` worktree path and records the target/compiler/profile in its manifest; its installed binary must be built at that path and verified from the generated manifest. The local Rust version and build overrides are absent from the archived local summary, so those conditions cannot yet be compared.

Existing evidence at this source SHA includes five completed local Arena matches with all days accepted and `fleet day plan clean=true`, recorded in Issue #326. The archived local official-server check at [branch 326](https://github.com/rebuildup/procon-2026/blob/326/docs/evidence/campaign-311/arena-1009/verify-5ddb153/auto7/summary.json) records CLI SHA256 `10172dc74ba13cae47ffc458967b7ebd552f99c1dc74b4d522b7f90b81feb8a2`, ten accepted days, and `fleet day plan clean=true`; the Cloud task does not contain the matching Mac binary or its full day-state/action fixture. The official server binary in `docs/evidence/campaign-311/official-server-parity` is Darwin arm64. Therefore exact action-plan parity between the two compiled binaries remains unverified. Before a Cloud live match, replay the same captured official setting/day-state through both target builds and compare submitted actions plus predicted position/fuel transitions; preserve the fixture and hashes without tokens.

## Start, stop, failure, and rollback

After the explicit handoff and all checks above, reload systemd. Start the oneshot service manually during a valid three-minute lobby to make the required single live match. The service blocks until the match completes:

```sh
systemctl daemon-reload
systemctl start procon-arena-runner.service
journalctl -u procon-arena-runner.service --since '15 minutes ago' --no-pager
```

Confirm exactly one `completed` entry in the persistent state and matching `runs.jsonl` evidence before enabling continuous operation. If the command returns `outside-window`, wait for the next lobby and start the oneshot inside its three-minute window; do not force a late registration. If the attempt fails or is ambiguous, leave the timer stopped and reconcile before any retry. After the first successful match:

```sh
systemctl enable --now procon-arena-runner.timer
systemctl status procon-arena-runner.timer
```

The timer then submits at each next slot, including after host reboot. The service has no automatic timeout or restart that could interrupt a long match; an unresolved attempt in persistent state blocks later slots. The timer itself is not installed, enabled, or started by this repository.

To stop future registrations, run `systemctl disable --now procon-arena-runner.timer`. Do not kill an active match during handoff. The oneshot service has no restart policy, and stopping the timer does not restart the current coordinator. Keep the state directory and evidence intact.

If a run exits nonzero, the evidence is missing or ambiguous, the host restarts during a match, the token disappears, or the binary/manifest mismatches, keep the timer stopped. Inspect the coordinator log and Arena UI without calling token-bearing `GET /setting`. A slot marked `started` or `halted` may already have registered. Once the operator has evidence of the actual outcome, record a manual resolution while the timer is stopped:

```sh
python3 /opt/procon-arena/arena_runner.py reconcile 2026-10-10T06:15:00Z \
  --outcome completed --reason 'Arena UI confirms match 1969 completed; evidence was recovered.'
```

Use `not_registered` only after confirming no registration occurred; never retry the old slot. If the outcome remains unknown, leave the journal active and do not resume. For a rollback, stop the timer, preserve logs/evidence, restore the previous binary plus its matching build manifest and runner config, then require a new explicit operator review before restarting. Do not switch candidate or binary under an unresolved attempt.
