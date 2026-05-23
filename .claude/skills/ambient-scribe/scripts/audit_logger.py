"""
Audit logger for ambient-scribe.

Writes append-only audit events conforming to assets/audit-event-schema.json.
Events are HIPAA-safe by construction: no transcript text, no SOAP content, no
medical record numbers. Linkage to a specific encounter is via a non-reversible
handle produced by phi_scrubber.encounter_handle().

Sinks:
    - "file": append JSONL to a configured path (default).
    - "stdout": emit JSONL to stdout — for local development only.
    - Custom sinks can be registered via register_sink().

Concurrency:
    File sink uses an exclusive append with fcntl on POSIX. For multi-host
    deployments, point at a shared, append-tolerant store (S3 with object lock,
    a write-only Kafka topic, etc.) by registering a custom sink.
"""

from __future__ import annotations

import json
import os
import sys
import threading
import uuid
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Protocol

try:  # POSIX-only; absence is tolerated.
    import fcntl  # type: ignore
except ImportError:  # pragma: no cover
    fcntl = None  # type: ignore


SCHEMA_VERSION = "1.0.0"


# ---------------------------------------------------------------------------
# Event model
# ---------------------------------------------------------------------------

@dataclass
class Counts:
    escalations_immediate: int = 0
    escalations_same_visit: int = 0
    escalations_follow_up: int = 0
    grounding_failures: int = 0
    unresolved_meds: int = 0
    verification_items: int = 0


@dataclass
class ChangeControl:
    pccp_version: str
    change_category: str  # one of: none|prompt|escalation_criteria|medication_vocab|scrubber_rules|other


@dataclass
class AuditEvent:
    encounter_handle: str
    transcript_sha256: str
    draft_sha256: str
    model_id: str
    skill_version: str
    prompt_hash: str
    counts: Counts = field(default_factory=Counts)
    user_handle: str | None = None
    change_control: ChangeControl | None = None
    dry_run: bool = False
    incident: bool = False
    incident_note: str | None = None
    event_id: str = field(default_factory=lambda: str(uuid.uuid4()))
    timestamp: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat(timespec="seconds"))
    schema_version: str = SCHEMA_VERSION

    def to_dict(self) -> dict[str, Any]:
        d = asdict(self)
        # Drop None to keep payloads minimal — optional fields are documented in schema.
        return {k: v for k, v in d.items() if v is not None}


# ---------------------------------------------------------------------------
# Sinks
# ---------------------------------------------------------------------------

class Sink(Protocol):
    def write(self, event: dict[str, Any]) -> None: ...


class FileSink:
    """Append-only JSONL sink. Uses fcntl for cross-process safety on POSIX."""

    def __init__(self, path: str | Path) -> None:
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()

    def write(self, event: dict[str, Any]) -> None:
        line = json.dumps(event, separators=(",", ":"), sort_keys=True) + "\n"
        with self._lock:
            with open(self.path, "a", encoding="utf-8") as f:
                if fcntl is not None:
                    fcntl.flock(f.fileno(), fcntl.LOCK_EX)
                try:
                    f.write(line)
                    f.flush()
                    os.fsync(f.fileno())
                finally:
                    if fcntl is not None:
                        fcntl.flock(f.fileno(), fcntl.LOCK_UN)


class StdoutSink:
    def write(self, event: dict[str, Any]) -> None:
        sys.stdout.write(json.dumps(event, separators=(",", ":"), sort_keys=True) + "\n")
        sys.stdout.flush()


_REGISTRY: dict[str, Callable[[], Sink]] = {}
_DEFAULT_SINK: Sink | None = None
_DEFAULT_LOCK = threading.Lock()


def register_sink(name: str, factory: Callable[[], Sink]) -> None:
    _REGISTRY[name] = factory


def configure(sink: Sink | str | None = None, *, path: str | Path | None = None) -> Sink:
    """Configure the module-level default sink.

    Examples:
        configure("file", path="/var/log/ambient-scribe/audit.jsonl")
        configure("stdout")
        configure(my_custom_sink_instance)
    """
    global _DEFAULT_SINK
    with _DEFAULT_LOCK:
        if isinstance(sink, str):
            if sink == "file":
                resolved = FileSink(path or os.environ.get("AMBIENT_SCRIBE_AUDIT_PATH", "./ambient-scribe-audit.jsonl"))
            elif sink == "stdout":
                resolved = StdoutSink()
            elif sink in _REGISTRY:
                resolved = _REGISTRY[sink]()
            else:
                raise ValueError(f"unknown sink: {sink!r}")
        elif sink is None:
            resolved = FileSink(path or os.environ.get("AMBIENT_SCRIBE_AUDIT_PATH", "./ambient-scribe-audit.jsonl"))
        else:
            resolved = sink
        _DEFAULT_SINK = resolved
        return resolved


def _sink() -> Sink:
    if _DEFAULT_SINK is None:
        return configure()
    return _DEFAULT_SINK


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def emit(event: AuditEvent) -> str:
    """Write an audit event. Returns the event_id."""
    payload = event.to_dict()
    _sink().write(payload)
    return event.event_id


def emit_incident(
    *,
    encounter_handle: str,
    model_id: str,
    skill_version: str,
    prompt_hash: str,
    note: str,
    transcript_sha256: str = "0" * 64,
    draft_sha256: str = "0" * 64,
) -> str:
    """Convenience: emit an incident event. ``note`` must be PHI-scrubbed by the caller."""
    ev = AuditEvent(
        encounter_handle=encounter_handle,
        transcript_sha256=transcript_sha256,
        draft_sha256=draft_sha256,
        model_id=model_id,
        skill_version=skill_version,
        prompt_hash=prompt_hash,
        incident=True,
        incident_note=note,
    )
    return emit(ev)
