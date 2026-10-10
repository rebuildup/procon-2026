#!/usr/bin/env python3
"""Fail-closed, single-host scheduler for one Arena match per cadence slot."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Mapping, Sequence


SOURCE_SHA = "5ddb153a16d44bb34c2c3ed5cea4aa8044f95bf0"
PRIMARY_CANDIDATE = "fleet:budget=official:budget_ms=10000"
FALLBACK_CANDIDATE = "anytime:budget_ms=5000:seed=42:budget=official"
ARENA_URL = "https://procon37arena.online"
CADENCE_SECONDS = 15 * 60
REGISTRATION_LEAD_SECONDS = 3 * 60
STATE_VERSION = 1

DEFAULT_BINARY = "/opt/procon-arena/procon"
DEFAULT_STATE_DIR = "/var/lib/procon-arena"
DEFAULT_OWNERSHIP_FILE = "/etc/procon-arena/ownership.json"
DEFAULT_RUNNER_CONFIG = "/etc/procon-arena/runner.json"
DEFAULT_BUILD_MANIFEST = "/opt/procon-arena/build.json"

SAFE_ENV_KEYS = {
    "PATH", "HOME", "LANG", "LC_ALL", "TMPDIR", "TZ",
    "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY",
    "http_proxy", "https_proxy", "all_proxy", "no_proxy",
    "SSL_CERT_FILE", "SSL_CERT_DIR", "CURL_CA_BUNDLE",
}


class RunnerError(RuntimeError):
    """A fail-closed precondition or execution error."""


class LockBusy(Exception):
    """Another invocation currently owns the single-run lock."""


@dataclass(frozen=True)
class Paths:
    binary: Path
    state_dir: Path
    ownership_file: Path
    runner_config: Path
    build_manifest: Path


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


def eligible_target(now: datetime) -> int | None:
    """Return the next cadence boundary only inside its T-3m lobby window."""
    if now.tzinfo is None:
        raise ValueError("clock must be timezone-aware")
    timestamp = now.timestamp()
    target = (int(timestamp) // CADENCE_SECONDS + 1) * CADENCE_SECONDS
    seconds_until = target - timestamp
    if 0 < seconds_until <= REGISTRATION_LEAD_SECONDS:
        return target
    return None


def _secure_regular_file(path: Path, *, secret_config: bool) -> bytes:
    try:
        file_stat = path.lstat()
    except FileNotFoundError as exc:
        raise RunnerError(f"required file missing: {path}") from exc
    if not stat.S_ISREG(file_stat.st_mode):
        raise RunnerError(f"required file is not a regular file: {path}")
    if secret_config and file_stat.st_mode & 0o037:
        raise RunnerError(f"sensitive config must deny group write/execute and all other access: {path}")
    return path.read_bytes()


def read_json(path: Path, *, secret_config: bool = False) -> dict:
    raw = _secure_regular_file(path, secret_config=secret_config)
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise RunnerError(f"invalid JSON config: {path}") from exc
    if not isinstance(value, dict):
        raise RunnerError(f"JSON config must be an object: {path}")
    return value


def require_ownership(path: Path) -> dict:
    handoff = read_json(path, secret_config=True)
    required_flags = (
        "enabled",
        "previous_operator_stopped",
        "previous_scheduler_stopped",
        "ownership_confirmed",
        "handoff_recorded_in_issue_317",
        "runner_validation_passed",
    )
    if any(handoff.get(flag) is not True for flag in required_flags):
        raise RunnerError("ownership handoff gate is closed")
    if not all(
        isinstance(handoff.get(field), str) and handoff[field].strip()
        for field in ("operator", "confirmed_at", "issue_317_url", "validation_ref")
    ):
        raise RunnerError("ownership handoff record is incomplete")
    if handoff["issue_317_url"] != "https://github.com/rebuildup/procon-2026/issues/317":
        raise RunnerError("ownership record must reference Issue #317")
    return handoff


def read_runner_config(path: Path) -> dict:
    config = read_json(path, secret_config=True)
    if config.get("candidate") not in (PRIMARY_CANDIDATE, FALLBACK_CANDIDATE):
        raise RunnerError("candidate must be the pinned primary or fallback")
    if config.get("fallback_candidate") != FALLBACK_CANDIDATE:
        raise RunnerError("fallback candidate does not match the pinned value")
    return config


def verify_build(binary: Path, manifest_path: Path) -> str:
    manifest = read_json(manifest_path)
    if manifest.get("source_commit_sha") != SOURCE_SHA:
        raise RunnerError("build source SHA does not match the pinned source")
    expected = manifest.get("binary_sha256")
    if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise RunnerError("build manifest has no valid binary SHA256")
    try:
        actual = hashlib.sha256(binary.read_bytes()).hexdigest()
    except OSError as exc:
        raise RunnerError("pinned coordinator binary is unavailable") from exc
    if actual != expected:
        raise RunnerError("coordinator binary SHA256 differs from its build manifest")
    return actual


def read_state(path: Path) -> dict:
    try:
        state_stat = path.lstat()
    except FileNotFoundError:
        return {"version": STATE_VERSION, "slots": {}, "active_slot": None}
    except OSError as exc:
        raise RunnerError("persistent state is unreadable; manual reconciliation required") from exc
    if not stat.S_ISREG(state_stat.st_mode) or state_stat.st_mode & 0o077:
        raise RunnerError("persistent state permissions or file type are unsafe")
    try:
        state = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise RunnerError("persistent state is unreadable; manual reconciliation required") from exc
    if not isinstance(state, dict) or state.get("version") != STATE_VERSION:
        raise RunnerError("persistent state version is unknown; manual reconciliation required")
    if not isinstance(state.get("slots"), dict) or "active_slot" not in state:
        raise RunnerError("persistent state is incomplete; manual reconciliation required")
    return state


def require_no_unresolved_attempt(state: dict) -> None:
    if state.get("active_slot") is not None:
        raise RunnerError("an earlier Arena attempt is unresolved; manual reconciliation required")
    allowed_statuses = {"started", "halted", "completed", "resolved"}
    unresolved = []
    for slot, record in state["slots"].items():
        if not isinstance(record, dict) or record.get("status") not in allowed_statuses:
            raise RunnerError("persistent slot history is invalid; manual reconciliation required")
        if record["status"] in {"started", "halted"}:
            unresolved.append(slot)
    if unresolved:
        raise RunnerError("an earlier Arena attempt is unresolved; manual reconciliation required")


def write_state(path: Path, state: dict) -> None:
    secure_state_directory(path.parent)
    encoded = (json.dumps(state, sort_keys=True, indent=2) + "\n").encode()
    fd, temporary_name = tempfile.mkstemp(prefix=".state-", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory_fd = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def minimal_child_env(source_sha: str, token: str | None = None) -> dict[str, str]:
    child_env = {key: value for key, value in os.environ.items() if key in SAFE_ENV_KEYS}
    child_env["TZ"] = "UTC"
    child_env["PROCON_ARENA_SERVER"] = ARENA_URL
    child_env["PROCON_SOURCE_COMMIT_SHA"] = source_sha
    if token is not None:
        child_env["PROCON_ARENA_TOKEN"] = token
    child_env["RUST_LOG"] = "info"
    return child_env


def _decode_output(value: bytes | str | None) -> str:
    if value is None:
        return ""
    return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value


def redact(text: str, token: str) -> str:
    # Coordinator output is already redacted. Keep a second boundary at the
    # Runner so an echoed token can never enter its persistent log.
    variants = {token}
    from urllib.parse import quote, quote_plus

    variants.add(quote(token, safe=""))
    variants.add(quote_plus(token, safe=""))
    for secret in sorted((item for item in variants if item), key=len, reverse=True):
        text = text.replace(secret, "[REDACTED]")
    return text


class StreamingRedactor:
    """Redact secret values while preserving safe output across read chunks."""

    def __init__(self, token: str) -> None:
        from urllib.parse import quote, quote_plus

        variants = {token, quote(token, safe=""), quote_plus(token, safe="")}
        self.secrets = tuple(sorted((item.encode() for item in variants if item), key=len, reverse=True))
        self.max_secret_length = max((len(item) for item in self.secrets), default=1)
        self.buffer = b""

    def feed(self, chunk: bytes) -> bytes:
        self.buffer += chunk
        stable_end = max(0, len(self.buffer) - self.max_secret_length + 1)
        output = bytearray()
        consumed = 0
        while consumed < stable_end:
            matches = [self.buffer.find(secret, consumed) for secret in self.secrets]
            matches = [index for index in matches if index >= 0 and index < stable_end]
            if not matches:
                break
            start = min(matches)
            secret = next(
                value for value in self.secrets
                if self.buffer.startswith(value, start)
            )
            output.extend(self.buffer[consumed:start])
            output.extend(b"[REDACTED]")
            consumed = start + len(secret)
        if consumed < stable_end:
            output.extend(self.buffer[consumed:stable_end])
            consumed = stable_end
        self.buffer = self.buffer[consumed:]
        return bytes(output)

    def finish(self) -> bytes:
        remaining = self.buffer
        self.buffer = b""
        for secret in self.secrets:
            remaining = remaining.replace(secret, b"[REDACTED]")
        return remaining


def stream_process(
    command: Sequence[str], *, env: Mapping[str, str], log_path: Path, token: str
) -> int:
    """Run coordinator while streaming only redacted output to a private log."""
    secure_state_directory(log_path.parent)
    flags = os.O_WRONLY | os.O_CREAT | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(log_path, flags, 0o600)
    os.fchmod(fd, 0o600)
    process = None
    try:
        with os.fdopen(fd, "ab", buffering=0) as output:
            output.write(b"--- coordinator output ---\n")
            process = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                env=dict(env),
                bufsize=0,
            )
            redactor = StreamingRedactor(token)
            assert process.stdout is not None
            while True:
                chunk = process.stdout.read(4096)
                if not chunk:
                    break
                safe = redactor.feed(chunk)
                if safe:
                    output.write(safe)
                    os.fsync(output.fileno())
            tail = redactor.finish()
            if tail:
                output.write(tail)
            output.flush()
            os.fsync(output.fileno())
            return process.wait()
    except BaseException:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        raise
    finally:
        if process is not None and process.stdout is not None:
            process.stdout.close()


def append_redacted_log(path: Path, stdout: bytes | str | None, stderr: bytes | str | None, token: str) -> None:
    secure_state_directory(path.parent)
    content = "--- stdout ---\n" + redact(_decode_output(stdout), token)
    content += "\n--- stderr ---\n" + redact(_decode_output(stderr), token)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    os.fchmod(fd, 0o600)
    try:
        with os.fdopen(fd, "a", encoding="utf-8") as stream:
            stream.write(content)
            if not content.endswith("\n"):
                stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        pass


def secure_state_directory(path: Path) -> None:
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    directory_stat = path.lstat()
    if not stat.S_ISDIR(directory_stat.st_mode) or directory_stat.st_mode & 0o077:
        raise RunnerError("state directory must be a private, non-symlink directory")
    if directory_stat.st_uid != os.geteuid():
        raise RunnerError("state directory must be owned by the Runner account")


def validate_new_evidence(path: Path, initial_size: int, candidate: str) -> dict:
    try:
        with path.open("rb") as stream:
            stream.seek(initial_size)
            new_lines = [line for line in stream.read().splitlines() if line.strip()]
    except OSError as exc:
        raise RunnerError("coordinator did not write the persistent Arena evidence") from exc
    if len(new_lines) != 1:
        raise RunnerError("expected exactly one new Arena evidence record")
    try:
        record = json.loads(new_lines[0])
    except json.JSONDecodeError as exc:
        raise RunnerError("new Arena evidence record is invalid JSON") from exc
    if (
        record.get("record_kind") != "arena-run"
        or record.get("status") != "completed"
        or record.get("source_commit_sha") != SOURCE_SHA
        or record.get("candidate_id") != candidate
        or not record.get("arena_match_id")
    ):
        raise RunnerError("new Arena evidence does not confirm one completed pinned match")
    return {
        "match_id": str(record["arena_match_id"]),
        "days_completed": record.get("days_completed"),
        "revisions": record.get("revisions"),
        "window_starts_at_unix": record.get("window_starts_at_unix"),
        "status": record["status"],
    }


def _slot_key(target_start: int) -> str:
    return datetime.fromtimestamp(target_start, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def load_paths() -> Paths:
    return Paths(
        binary=Path(os.environ.get("ARENA_RUNNER_BINARY", DEFAULT_BINARY)),
        state_dir=Path(os.environ.get("ARENA_RUNNER_STATE_DIR", DEFAULT_STATE_DIR)),
        ownership_file=Path(os.environ.get("ARENA_RUNNER_OWNERSHIP", DEFAULT_OWNERSHIP_FILE)),
        runner_config=Path(os.environ.get("ARENA_RUNNER_CONFIG", DEFAULT_RUNNER_CONFIG)),
        build_manifest=Path(os.environ.get("ARENA_RUNNER_BUILD_MANIFEST", DEFAULT_BUILD_MANIFEST)),
    )


class ArenaRunner:
    def __init__(
        self,
        paths: Paths,
        *,
        clock: Callable[[], datetime] = utc_now,
        execute: Callable[..., subprocess.CompletedProcess] = subprocess.run,
        execute_stream: Callable[..., int] = stream_process,
        spawn: Callable[..., subprocess.CompletedProcess] = subprocess.run,
    ) -> None:
        self.paths = paths
        self.clock = clock
        self.execute = execute
        self.execute_stream = execute_stream
        self.spawn = spawn

    def _acquire_lock(self):
        secure_state_directory(self.paths.state_dir)
        lock_path = self.paths.state_dir / "runner.lock"
        fd = os.open(
            lock_path,
            os.O_CREAT | os.O_RDWR | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        lock_stat = os.fstat(fd)
        if not stat.S_ISREG(lock_stat.st_mode) or lock_stat.st_mode & 0o077:
            os.close(fd)
            raise RunnerError("runner lock file permissions or type are unsafe")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            os.close(fd)
            raise LockBusy from exc
        return fd

    def _preflight(self) -> None:
        command = [str(self.paths.binary), "coordinator", "--help"]
        try:
            result = self.execute(
                command,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                env=minimal_child_env(SOURCE_SHA),
                timeout=30,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise RunnerError("pinned coordinator preflight failed") from exc
        output = _decode_output(result.stdout)
        required_flags = ("--arena", "--next", "--exit-after-match", "--candidate", "--kinds-auto")
        if result.returncode != 0 or any(flag not in output for flag in required_flags):
            raise RunnerError("pinned coordinator preflight does not expose the required Arena flags")

    def run_once(self) -> str:
        try:
            lock_fd = self._acquire_lock()
        except LockBusy:
            return "locked"
        try:
            require_ownership(self.paths.ownership_file)
            config = read_runner_config(self.paths.runner_config)
            binary_sha = verify_build(self.paths.binary, self.paths.build_manifest)
            self._preflight()
            token = os.environ.get("PROCON_ARENA_TOKEN", "")
            if not token.strip():
                raise RunnerError("PROCON_ARENA_TOKEN is missing")

            now = self.clock()
            target_start = eligible_target(now)
            if target_start is None:
                return "outside-window"
            slot = _slot_key(target_start)
            state_path = self.paths.state_dir / "state.json"
            state = read_state(state_path)
            require_no_unresolved_attempt(state)
            if slot in state["slots"]:
                return "already-recorded"

            candidate = config["candidate"]
            kinds_args = ["--kinds-auto"] if candidate == PRIMARY_CANDIDATE else []
            evidence_path = self.paths.state_dir / "runs.jsonl"
            try:
                evidence_size = evidence_path.stat().st_size
            except FileNotFoundError:
                evidence_size = 0
            attempt = {
                "status": "started",
                "target_start_unix": target_start,
                "started_at": now.astimezone(timezone.utc).isoformat(),
                "source_commit_sha": SOURCE_SHA,
                "binary_sha256": binary_sha,
                "candidate": candidate,
                "kinds": "--kinds-auto" if kinds_args else "default",
                "evidence_path": str(evidence_path),
            }
            state["slots"][slot] = attempt
            state["active_slot"] = slot
            write_state(state_path, state)

            command = [
                str(self.paths.binary), "coordinator",
                "--arena", "--next", "--exit-after-match",
                "--candidate", candidate, *kinds_args,
                "--arena-evidence", str(evidence_path),
            ]
            try:
                log_path = self.paths.state_dir / "logs" / f"{slot}.log"
                returncode = self.execute_stream(
                    command,
                    env=minimal_child_env(SOURCE_SHA, token),
                    log_path=log_path,
                    token=token,
                )
                if returncode != 0:
                    raise RunnerError("coordinator exited unsuccessfully; slot requires reconciliation")
                evidence = validate_new_evidence(evidence_path, evidence_size, candidate)
                attempt.update({
                    "status": "completed",
                    "completed_at": self.clock().astimezone(timezone.utc).isoformat(),
                    "match": evidence,
                    "log_path": str(log_path),
                })
                state["active_slot"] = None
                write_state(state_path, state)
                return "completed"
            except BaseException as exc:
                # Keep active_slot set. The runner may have registered before
                # the failure became visible, so automatic retry is unsafe.
                attempt["status"] = "halted"
                attempt["halt_reason"] = type(exc).__name__
                attempt["halted_at"] = self.clock().astimezone(timezone.utc).isoformat()
                write_state(state_path, state)
                if isinstance(exc, (KeyboardInterrupt, SystemExit)):
                    raise
                raise RunnerError("Arena attempt halted; reconcile before the next slot") from None
        finally:
            try:
                fcntl.flock(lock_fd, fcntl.LOCK_UN)
            finally:
                os.close(lock_fd)

    def run_gated(self) -> str:
        """Check handoff and slot before asking Infisical for the Arena token."""
        require_ownership(self.paths.ownership_file)
        read_runner_config(self.paths.runner_config)
        verify_build(self.paths.binary, self.paths.build_manifest)
        self._preflight()
        target_start = eligible_target(self.clock())
        if target_start is None:
            return "outside-window"
        try:
            lock_fd = self._acquire_lock()
        except LockBusy:
            return "locked"
        try:
            state = read_state(self.paths.state_dir / "state.json")
            require_no_unresolved_attempt(state)
            if _slot_key(target_start) in state["slots"]:
                return "already-recorded"
        finally:
            try:
                fcntl.flock(lock_fd, fcntl.LOCK_UN)
            finally:
                os.close(lock_fd)

        # Infisical receives only its own configured Machine Identity vars.
        # The nested env removes them before starting the token-bearing runner.
        identity_keys = {
            "INFISICAL_DOMAIN", "INFISICAL_PROJECT_ID", "INFISICAL_CLIENT_ID",
            "INFISICAL_CLIENT_SECRET", "INFISICAL_UNIVERSAL_AUTH_CLIENT_ID",
            "INFISICAL_UNIVERSAL_AUTH_CLIENT_SECRET",
        }
        identity_env = {
            key: value for key, value in os.environ.items()
            if key in SAFE_ENV_KEYS or key in identity_keys or key.startswith("ARENA_RUNNER_")
        }
        client_id = identity_env.get("INFISICAL_UNIVERSAL_AUTH_CLIENT_ID") or identity_env.get("INFISICAL_CLIENT_ID")
        client_secret = identity_env.get("INFISICAL_UNIVERSAL_AUTH_CLIENT_SECRET") or identity_env.get("INFISICAL_CLIENT_SECRET")
        project_id = identity_env.get("INFISICAL_PROJECT_ID")
        if not all((client_id, client_secret, project_id, identity_env.get("INFISICAL_DOMAIN"))):
            raise RunnerError("Infisical Machine Identity settings are incomplete")
        identity_env["INFISICAL_UNIVERSAL_AUTH_CLIENT_ID"] = client_id
        identity_env["INFISICAL_UNIVERSAL_AUTH_CLIENT_SECRET"] = client_secret
        infisical = ["/usr/bin/env", "infisical"]
        login_command = infisical + [
            "login", "--method=universal-auth", "--save-as=arena-runner",
        ]
        try:
            login_result = self.spawn(
                login_command,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=identity_env,
                timeout=30,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise RunnerError("Infisical Machine Identity login failed") from exc
        if login_result.returncode != 0:
            raise RunnerError("Infisical Machine Identity login failed")

        command = infisical + [
            "run", "--profile=arena-runner", f"--projectId={project_id}",
            "--env=dev", "--path=/arena-runner", "--include-imports=false",
            "--secret-overriding=false", "--silent", "--",
            "/usr/bin/env",
            "-u", "INFISICAL_CLIENT_ID",
            "-u", "INFISICAL_CLIENT_SECRET",
            "-u", "INFISICAL_UNIVERSAL_AUTH_CLIENT_ID",
            "-u", "INFISICAL_UNIVERSAL_AUTH_CLIENT_SECRET",
            "-u", "INFISICAL_PROJECT_ID",
            "-u", "INFISICAL_DOMAIN",
            "-u", "INFISICAL_TOKEN",
            "/usr/bin/python3", str(Path(__file__).resolve()), "once",
        ]
        try:
            result = self.spawn(
                command,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=identity_env,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise RunnerError("Infisical invocation failed") from exc
        if result.returncode != 0:
            raise RunnerError("Infisical injection or the one-match run failed")
        output = _decode_output(result.stdout)
        for result_name in ("completed", "outside-window", "locked", "already-recorded"):
            if output.rstrip().endswith(f"arena-runner: {result_name}"):
                return result_name
        raise RunnerError("one-match runner returned no recognized status")

    def reconcile(self, slot: str, outcome: str, reason: str) -> None:
        if outcome not in {"completed", "not_registered", "abandoned"}:
            raise RunnerError("reconciliation outcome must be completed, not_registered, or abandoned")
        if len(reason.strip()) < 12:
            raise RunnerError("reconciliation requires a concise evidence-based reason")
        if re.search(r"[A-Za-z0-9_-]{48,}", reason) or re.search(
            r"(?i)\b(bearer|token|secret|password)\b\s*[:=]", reason
        ):
            raise RunnerError("reconciliation reason contains credential-like material")
        try:
            lock_fd = self._acquire_lock()
        except LockBusy:
            raise RunnerError("runner is active; stop it before reconciling") from None
        try:
            state_path = self.paths.state_dir / "state.json"
            state = read_state(state_path)
            if state.get("active_slot") != slot:
                raise RunnerError("requested slot is not the unresolved active slot")
            attempt = state["slots"].get(slot)
            if not isinstance(attempt, dict) or attempt.get("status") not in {"started", "halted"}:
                raise RunnerError("slot is not in a reconcilable state")
            attempt["status"] = "resolved"
            attempt["reconciliation"] = {
                "outcome": outcome,
                "reason": reason.strip(),
                "recorded_at": self.clock().astimezone(timezone.utc).isoformat(),
            }
            state["active_slot"] = None
            write_state(state_path, state)
        finally:
            try:
                fcntl.flock(lock_fd, fcntl.LOCK_UN)
            finally:
                os.close(lock_fd)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("gated-run", "once", "reconcile"))
    parser.add_argument("slot", nargs="?", help="UTC slot key, e.g. 2026-10-10T06:15:00Z")
    parser.add_argument("--outcome", choices=("completed", "not_registered", "abandoned"))
    parser.add_argument("--reason", help="Evidence-based operator reconciliation note")
    args = parser.parse_args(argv)
    if args.command != "gated-run":
        for name in tuple(os.environ):
            if name.startswith("INFISICAL_"):
                os.environ.pop(name, None)
    runner = ArenaRunner(load_paths())
    try:
        if args.command == "gated-run":
            result = runner.run_gated()
        elif args.command == "once":
            result = runner.run_once()
        else:
            if not args.slot or not args.outcome or not args.reason:
                raise RunnerError("reconcile requires SLOT, --outcome, and --reason")
            runner.reconcile(args.slot, args.outcome, args.reason)
            result = "reconciled"
    except RunnerError as exc:
        print(f"arena-runner: {exc}", file=sys.stderr)
        return 1
    print(f"arena-runner: {result}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
