# CLI vs Official Server — Manual Verification

This is the manual test procedure that reproduces the **kind-window / `POST /agent` ordering bug** (issue #156) against the actual contest server binary, and confirms the fix.

## Bug summary

The official `procon-server-linux-amd64` server has a **kind-submission
window** that opens at server start and closes at `kind-deadline`
(default 5 s; we exercised with 60 s). During that window:

- `GET /setting` returns `{"startsAt": 0, ...}` (start not yet
  announced)
- `POST /agent` returns `200 OK`

After the deadline, the server announces the start and the kind window
closes:

- `GET /setting` returns `{"startsAt": <unix>, ...}`
- `POST /agent` returns `403 AccessTimeError`

Pre-fix CLI ordering:

```
Phase 1: poll GET /setting until startsAt > 0  (window already closed!)
Phase 2: POST /agent                           -> 403, exit with error
```

Fixed CLI ordering (this PR):

```
Phase 1: poll GET /setting until server responsive (any startsAt)
Phase 2: POST /agent with retry-on-403         -> 200, kinds accepted
Phase 3: wire -> engine conversion
Phase 4: poll GET /setting until startsAt > 0
Phase 5: per-day loop
```

## Setup

1. Download `回答システムに関する情報・簡易版の回答用サーバー_1.zip`
   from the contest site.
3. Extract to `/tmp/official-server/`.
3. Edit `config/example.json` so `daySeconds=[10,10,10,10]` and
   `daySteps=[6,6,6,6]` (faster match; the loop runs ~40s instead of
   the default ~6 minutes).
4. Build the CLI from this branch:

   ```bash
   cargo build --release -p cli
   ```

## Reproduce the bug (pre-fix, optional)

If you have a checkout of the pre-fix branch (e.g. `origin/release-0-2-0`
HEAD before this PR), build it and use it as `procon-pre-fix` in step 5.

Expected output (pre-fix, with `kind-deadline=60s`):

```
[server] INFO listening addr=:8080
[server] INFO agent kinds closed startsAt=1789890581
[server] INFO match started startsAt=1789890581
[CLI]    error: coordinator loop: POST /agent failed: access time error: agent kind submission is closed
```

This is the failure the PR fixes.

## Run the fix (post-fix)

In one terminal:

```bash
cd /tmp/official-server/server-bin
./procon-server-linux-amd64 \
    -config ../config/example.json \
    -addr :8080 \
    -kind-deadline 60s \
    -match-start-delay 1s
```

In a second terminal:

```bash
PROCON_SERVER=http://localhost:8080 \
PROCON_TOKEN=token-p0 \
./target/release/procon coordinator --solver loop
```

## Expected (post-fix)

- Within ~60 s, CLI logs `coordinator loop strategy selected solver="loop"`.
- Server logs `agent kinds submitted team=0` (within the kind window —
  the bug-fixed ordering ensures this).
- After `~10 s × 4 days = ~40 s` of day progression, server logs
  `day advanced day=1`, `day=2`, `day=3` and the match ends.
- CLI exits with no error and prints the per-day
  `{"days_completed":4,"revisions":[...]}` summary.

The match should complete cleanly. If you see
`access time error: agent kind submission is closed`, the bug has
regressed — please file an issue with the server log excerpt.

## Tuning the deadline for faster iteration

The default `kind-deadline` of 5 s is tight for an interactive test. Use
`-kind-deadline 60s` (as shown above) to give yourself time to observe
both server and CLI logs. The CLI's retry budget on `403 AccessTimeError`
is `30 attempts × 500ms = 15s` (see
`CoordinatorLoopConfig::kind_submit_max_attempts` and
`kind_submit_backoff`), so it comfortably tolerates either deadline.