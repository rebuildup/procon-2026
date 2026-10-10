from __future__ import annotations

import hashlib
import fcntl
import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace

from arena_runner import (
    ARENA_URL,
    FALLBACK_CANDIDATE,
    PRIMARY_CANDIDATE,
    SOURCE_SHA,
    ArenaRunner,
    Paths,
    RunnerError,
    StreamingRedactor,
    append_redacted_log,
    eligible_target,
    minimal_child_env,
    stream_process,
)


def epoch(value: str) -> int:
    return int(datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp())


class ArenaRunnerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.state_dir = self.root / "state"
        self.binary = self.root / "procon"
        self.binary.write_bytes(b"mock coordinator binary")
        self.binary.chmod(0o700)
        binary_hash = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        self.manifest = self.root / "build.json"
        self.manifest.write_text(json.dumps({"source_commit_sha": SOURCE_SHA, "binary_sha256": binary_hash}))
        self.runner_config = self.root / "runner.json"
        self.write_runner_config(PRIMARY_CANDIDATE)
        self.ownership = self.root / "ownership.json"
        self.write_ownership(enabled=False)
        self.now = datetime(2026, 10, 10, 6, 12, tzinfo=timezone.utc)
        self.execute_calls = []
        self.spawn_calls = []

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write_runner_config(self, candidate: str) -> None:
        self.runner_config.write_text(json.dumps({"candidate": candidate, "fallback_candidate": FALLBACK_CANDIDATE}))
        self.runner_config.chmod(0o600)

    def write_ownership(self, *, enabled: bool) -> None:
        data = {
            "enabled": enabled,
            "previous_operator_stopped": enabled,
            "previous_scheduler_stopped": enabled,
            "ownership_confirmed": enabled,
            "handoff_recorded_in_issue_317": enabled,
            "runner_validation_passed": enabled,
            "operator": "test-operator" if enabled else "",
            "confirmed_at": "2026-10-10T06:00:00Z" if enabled else "",
            "issue_317_url": "https://github.com/rebuildup/procon-2026/issues/317",
            "validation_ref": "mock-test" if enabled else "",
        }
        self.ownership.write_text(json.dumps(data))
        self.ownership.chmod(0o600)

    def paths(self) -> Paths:
        return Paths(self.binary, self.state_dir, self.ownership, self.runner_config, self.manifest)

    def fake_execute(self, command, **kwargs):
        self.execute_calls.append((command, kwargs))
        if command[-1] == "--help":
            return SimpleNamespace(returncode=0, stdout=b"--arena --next --exit-after-match --candidate --kinds-auto")
        evidence_path = Path(command[command.index("--arena-evidence") + 1])
        record = {
            "record_kind": "arena-run",
            "status": "completed",
            "source_commit_sha": SOURCE_SHA,
            "candidate_id": command[command.index("--candidate") + 1],
            "arena_match_id": "mock-1969",
            "days_completed": 8,
            "revisions": [1, 2, 3],
            "window_starts_at_unix": epoch("2026-10-10T06:15:00Z"),
        }
        with evidence_path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(record) + "\n")
        return SimpleNamespace(returncode=0, stdout=b"safe output", stderr=b"")

    def runner(self, execute=None, spawn=None):
        executor = execute or self.fake_execute

        def fake_stream(command, *, env, log_path, token):
            result = executor(
                command,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=env,
            )
            append_redacted_log(log_path, result.stdout, result.stderr, token)
            return result.returncode

        return ArenaRunner(
            self.paths(), clock=lambda: self.now, execute=executor, execute_stream=fake_stream,
            spawn=spawn or self.fake_spawn,
        )

    def fake_spawn(self, command, **kwargs):
        self.spawn_calls.append((command, kwargs))
        if "login" in command:
            return SimpleNamespace(returncode=0, stdout=b"login ok", stderr=b"")
        return SimpleNamespace(returncode=0, stdout=b"arena-runner: outside-window", stderr=b"")

    def test_only_runs_inside_three_minute_opening_window(self):
        self.assertEqual(epoch("2026-10-10T06:15:00Z"), eligible_target(datetime(2026, 10, 10, 6, 12, tzinfo=timezone.utc)))
        self.assertEqual(epoch("2026-10-10T06:15:00Z"), eligible_target(datetime(2026, 10, 10, 6, 14, 59, tzinfo=timezone.utc)))
        self.assertIsNone(eligible_target(datetime(2026, 10, 10, 6, 15, tzinfo=timezone.utc)))
        self.assertIsNone(eligible_target(datetime(2026, 10, 10, 6, 16, tzinfo=timezone.utc)))
        self.assertIsNone(eligible_target(datetime(2026, 10, 10, 6, 26, 59, tzinfo=timezone.utc)))

    def test_disabled_ownership_gate_never_invokes_coordinator(self):
        with self.assertRaisesRegex(RunnerError, "ownership handoff gate"):
            self.runner().run_once()
        self.assertEqual([], self.execute_calls)

    def test_closed_gate_does_not_fetch_secret_from_infisical(self):
        with self.assertRaisesRegex(RunnerError, "ownership handoff gate"):
            self.runner().run_gated()
        self.assertEqual([], self.spawn_calls)

    def test_missed_lobby_does_not_authenticate_to_infisical(self):
        self.write_ownership(enabled=True)
        self.now = datetime(2026, 10, 10, 6, 16, tzinfo=timezone.utc)
        self.assertEqual("outside-window", self.runner().run_gated())
        self.assertEqual([], self.spawn_calls)

    def test_gated_runner_uses_identity_env_without_credentials_in_args(self):
        self.write_ownership(enabled=True)
        os.environ["INFISICAL_CLIENT_ID"] = "mock-client-id"
        os.environ["INFISICAL_CLIENT_SECRET"] = "mock-client-secret"
        os.environ["INFISICAL_PROJECT_ID"] = "mock-project-id"
        os.environ["INFISICAL_DOMAIN"] = "https://secrets.example.invalid"
        try:
            self.assertEqual("outside-window", self.runner().run_gated())
        finally:
            for name in (
                "INFISICAL_CLIENT_ID", "INFISICAL_CLIENT_SECRET",
                "INFISICAL_PROJECT_ID", "INFISICAL_DOMAIN",
            ):
                os.environ.pop(name, None)
        login_command, login_kwargs = self.spawn_calls[0]
        run_command, run_kwargs = self.spawn_calls[1]
        self.assertIn("--method=universal-auth", login_command)
        self.assertNotIn("mock-client-secret", login_command)
        self.assertEqual("mock-client-secret", login_kwargs["env"]["INFISICAL_UNIVERSAL_AUTH_CLIENT_SECRET"])
        self.assertIn("--path=/arena-runner", run_command)
        self.assertIn("--include-imports=false", run_command)
        self.assertIn("--secret-overriding=false", run_command)
        self.assertNotIn("PROCON_ARENA_TOKEN", run_command)
        self.assertNotIn("PROCON_ARENA_TOKEN", run_kwargs["env"])

    def test_success_persists_one_completed_match_and_sanitizes_environment(self):
        self.write_ownership(enabled=True)
        os.environ["PROCON_ARENA_TOKEN"] = "test-only-token"
        os.environ["INFISICAL_CLIENT_SECRET"] = "must-not-propagate"
        try:
            self.assertEqual("completed", self.runner().run_once())
        finally:
            os.environ.pop("PROCON_ARENA_TOKEN", None)
            os.environ.pop("INFISICAL_CLIENT_SECRET", None)
        state = json.loads((self.state_dir / "state.json").read_text())
        self.assertIsNone(state["active_slot"])
        slot = state["slots"]["2026-10-10T06:15:00Z"]
        self.assertEqual("completed", slot["status"])
        self.assertEqual("mock-1969", slot["match"]["match_id"])
        coordinator = self.execute_calls[1]
        command, kwargs = coordinator
        self.assertIn("--exit-after-match", command)
        self.assertIn("--kinds-auto", command)
        self.assertEqual(PRIMARY_CANDIDATE, command[command.index("--candidate") + 1])
        self.assertEqual("test-only-token", kwargs["env"]["PROCON_ARENA_TOKEN"])
        self.assertNotIn("INFISICAL_CLIENT_SECRET", kwargs["env"])
        self.assertNotIn("PROCON_TOKEN", kwargs["env"])

    def test_same_slot_restart_does_not_reregister_completed_match(self):
        self.write_ownership(enabled=True)
        os.environ["PROCON_ARENA_TOKEN"] = "test-only-token"
        try:
            self.assertEqual("completed", self.runner().run_once())
            calls_after_first = sum(command[-1] != "--help" for command, _ in self.execute_calls)
            self.assertEqual("already-recorded", self.runner().run_once())
            coordinator_calls = sum(command[-1] != "--help" for command, _ in self.execute_calls)
            self.assertEqual(calls_after_first, coordinator_calls)
        finally:
            os.environ.pop("PROCON_ARENA_TOKEN", None)

    def test_uncertain_prior_attempt_blocks_later_slots_after_restart(self):
        self.write_ownership(enabled=True)
        self.state_dir.mkdir(mode=0o700)
        state_file = self.state_dir / "state.json"
        state_file.write_text(json.dumps({
            "version": 1,
            "active_slot": "2026-10-10T06:15:00Z",
            "slots": {"2026-10-10T06:15:00Z": {"status": "started"}},
        }))
        state_file.chmod(0o600)
        os.environ["PROCON_ARENA_TOKEN"] = "test-only-token"
        self.now = datetime(2026, 10, 10, 6, 27, tzinfo=timezone.utc)
        try:
            with self.assertRaisesRegex(RunnerError, "earlier Arena attempt"):
                self.runner().run_once()
        finally:
            os.environ.pop("PROCON_ARENA_TOKEN", None)

    def test_started_history_blocks_even_if_active_pointer_is_missing(self):
        self.write_ownership(enabled=True)
        self.state_dir.mkdir(mode=0o700)
        state_file = self.state_dir / "state.json"
        state_file.write_text(json.dumps({
            "version": 1,
            "active_slot": None,
            "slots": {"2026-10-10T06:15:00Z": {"status": "started"}},
        }))
        state_file.chmod(0o600)
        os.environ["PROCON_ARENA_TOKEN"] = "test-only-token"
        try:
            with self.assertRaisesRegex(RunnerError, "earlier Arena attempt"):
                self.runner().run_once()
        finally:
            os.environ.pop("PROCON_ARENA_TOKEN", None)

    def test_nonzero_exit_is_durable_halt_and_never_automatically_retried(self):
        self.write_ownership(enabled=True)
        os.environ["PROCON_ARENA_TOKEN"] = "test-only-token"

        def failed(command, **kwargs):
            if command[-1] == "--help":
                return SimpleNamespace(returncode=0, stdout=b"--arena --next --exit-after-match --candidate --kinds-auto")
            return SimpleNamespace(returncode=1, stdout=b"", stderr=b"failure")

        try:
            with self.assertRaisesRegex(RunnerError, "reconcile"):
                self.runner(failed).run_once()
            self.now = datetime(2026, 10, 10, 6, 27, tzinfo=timezone.utc)
            with self.assertRaisesRegex(RunnerError, "earlier Arena attempt"):
                self.runner(failed).run_once()
        finally:
            os.environ.pop("PROCON_ARENA_TOKEN", None)
        state = json.loads((self.state_dir / "state.json").read_text())
        self.assertEqual("halted", state["slots"]["2026-10-10T06:15:00Z"]["status"])

    def test_missing_token_fails_before_durable_claim(self):
        self.write_ownership(enabled=True)
        os.environ.pop("PROCON_ARENA_TOKEN", None)
        with self.assertRaisesRegex(RunnerError, "PROCON_ARENA_TOKEN is missing"):
            self.runner().run_once()
        self.assertFalse((self.state_dir / "state.json").exists())

    def test_lock_contention_skips_without_starting_another_coordinator(self):
        self.write_ownership(enabled=True)
        self.state_dir.mkdir(mode=0o700)
        lock_fd = os.open(self.state_dir / "runner.lock", os.O_CREAT | os.O_RDWR, 0o600)
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        os.environ["PROCON_ARENA_TOKEN"] = "test-only-token"
        try:
            self.assertEqual("locked", self.runner().run_once())
            self.assertEqual([], self.execute_calls)
        finally:
            os.environ.pop("PROCON_ARENA_TOKEN", None)
            fcntl.flock(lock_fd, fcntl.LOCK_UN)
            os.close(lock_fd)

    def test_only_pinned_fallback_can_be_selected(self):
        self.write_runner_config(FALLBACK_CANDIDATE)
        config = json.loads(self.runner_config.read_text())
        self.assertEqual(FALLBACK_CANDIDATE, config["candidate"])
        self.assertEqual(ARENA_URL, minimal_child_env(SOURCE_SHA)["PROCON_ARENA_SERVER"])

    def test_fallback_does_not_force_kinds_auto(self):
        self.write_ownership(enabled=True)
        self.write_runner_config(FALLBACK_CANDIDATE)
        os.environ["PROCON_ARENA_TOKEN"] = "test-only-token"
        try:
            self.assertEqual("completed", self.runner().run_once())
        finally:
            os.environ.pop("PROCON_ARENA_TOKEN", None)
        command = self.execute_calls[-1][0]
        self.assertEqual(FALLBACK_CANDIDATE, command[command.index("--candidate") + 1])
        self.assertNotIn("--kinds-auto", command)

    def test_reconciliation_is_explicit_and_keeps_original_attempt(self):
        self.state_dir.mkdir(mode=0o700)
        slot = "2026-10-10T06:15:00Z"
        state_file = self.state_dir / "state.json"
        state_file.write_text(json.dumps({
            "version": 1,
            "active_slot": slot,
            "slots": {slot: {"status": "halted", "halt_reason": "RunnerError"}},
        }))
        state_file.chmod(0o600)
        self.runner().reconcile(slot, "not_registered", "Arena UI confirmed no player joined this slot.")
        state = json.loads((self.state_dir / "state.json").read_text())
        self.assertIsNone(state["active_slot"])
        self.assertEqual("resolved", state["slots"][slot]["status"])
        self.assertEqual("not_registered", state["slots"][slot]["reconciliation"]["outcome"])

    def test_reconciliation_rejects_credential_shaped_reason(self):
        with self.assertRaisesRegex(RunnerError, "credential-like"):
            self.runner().reconcile(
                "2026-10-10T06:15:00Z",
                "abandoned",
                "credential aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )

    def test_logs_redact_token_even_if_child_echoes_it(self):
        self.write_ownership(enabled=True)
        os.environ["PROCON_ARENA_TOKEN"] = "test-only-token"

        def echoing(command, **kwargs):
            if command[-1] == "--help":
                return SimpleNamespace(returncode=0, stdout=b"--arena --next --exit-after-match --candidate --kinds-auto")
            evidence_path = Path(command[command.index("--arena-evidence") + 1])
            record = {
                "record_kind": "arena-run", "status": "completed", "source_commit_sha": SOURCE_SHA,
                "candidate_id": PRIMARY_CANDIDATE, "arena_match_id": "mock-1969",
            }
            evidence_path.write_text(json.dumps(record) + "\n")
            return SimpleNamespace(returncode=0, stdout=b"test-only-token", stderr=b"")

        try:
            self.runner(echoing).run_once()
        finally:
            os.environ.pop("PROCON_ARENA_TOKEN", None)
        log = (self.state_dir / "logs" / "2026-10-10T06:15:00Z.log").read_text()
        self.assertNotIn("test-only-token", log)
        self.assertIn("[REDACTED]", log)

    def test_streaming_redactor_handles_secret_split_between_chunks(self):
        redactor = StreamingRedactor("test-only-token")
        output = redactor.feed(b"before test-only-")
        output += redactor.feed(b"token after")
        output += redactor.finish()
        self.assertEqual(b"before [REDACTED] after", output)

    def test_real_stream_process_never_persists_child_echo_of_token(self):
        log_path = self.root / "private-logs" / "runner.log"
        code = "import sys; sys.stdout.write('prefix test-only-token suffix'); sys.stdout.flush()"
        result = stream_process(
            [sys.executable, "-c", code],
            env={"PATH": os.environ.get("PATH", "/usr/bin:/bin")},
            log_path=log_path,
            token="test-only-token",
        )
        self.assertEqual(0, result)
        log = log_path.read_text()
        self.assertNotIn("test-only-token", log)
        self.assertIn("[REDACTED]", log)


if __name__ == "__main__":
    unittest.main()
