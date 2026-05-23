"""
PHI scrubber for ambient-scribe.

Two modes:
    - safe_harbor: redact all 18 HIPAA Safe Harbor identifiers; output is suitable
      for non-clinical destinations (debug logs, telemetry, eval datasets).
    - encounter_handle: produce a stable, non-reversible hash for the encounter so
      audit events can be linked without exposing identity.

This is the canonical implementation. Do not write your own redactor. If you find
a gap, file a change request and update this script — do not patch in place
inside another script.

Limitations (be honest in the SRA):
    - Regex/heuristic-based scrubbing is not perfect. Free-text names that don't
      match the heuristics can leak through.
    - For high-assurance scrubbing of large free-text corpora, layer this with a
      named-entity recognition model. The skill exposes the seam; the deploying
      organization decides on the second layer.
"""

from __future__ import annotations

import hashlib
import hmac
import os
import re
from dataclasses import dataclass
from typing import Iterable

# ---------------------------------------------------------------------------
# Patterns
# ---------------------------------------------------------------------------

# Order matters: more specific patterns first.
_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("SSN", re.compile(r"\b\d{3}-\d{2}-\d{4}\b")),
    ("MRN", re.compile(r"\b(?:MRN|mrn|Medical Record(?: Number)?)[:\s#]*([A-Z0-9-]{4,})\b")),
    ("PHONE", re.compile(r"\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b")),
    ("FAX", re.compile(r"\bfax[:\s]+(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b", re.IGNORECASE)),
    ("EMAIL", re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")),
    ("URL", re.compile(r"\bhttps?://[^\s<>\"]+")),
    ("IP", re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")),
    # Dates more specific than year. Captures common formats; not exhaustive.
    ("DATE", re.compile(
        r"\b(?:\d{1,2}[/-]\d{1,2}[/-]\d{2,4}"
        r"|\d{4}-\d{2}-\d{2}"
        r"|(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\.?\s+\d{1,2}(?:,\s*\d{4})?)\b"
    )),
    # Account/license/device-style tokens (long alphanumeric runs with mixed digits).
    ("IDNUM", re.compile(r"\b(?=[A-Z0-9-]*\d)[A-Z0-9-]{6,}\b")),
    # ZIP — first three digits allowed under Safe Harbor for most ZIP3s, but the
    # safest default is to redact full 5/9-digit ZIPs entirely.
    ("ZIP", re.compile(r"\b\d{5}(?:-\d{4})?\b")),
    # Street-style addresses — heuristic.
    ("ADDRESS", re.compile(
        r"\b\d{1,6}\s+(?:[A-Z][a-zA-Z]*\s+){0,4}"
        r"(?:Street|St|Avenue|Ave|Road|Rd|Boulevard|Blvd|Drive|Dr|Lane|Ln|Way|Court|Ct|Place|Pl)\b\.?",
        re.IGNORECASE,
    )),
]

# Names: heuristic — sequences of two or more capitalized words. Conservative
# defaults err on the side of over-redaction in non-clinical destinations.
_NAME_PATTERN = re.compile(r"\b(?:Mr|Mrs|Ms|Dr|Mx)\.?\s+[A-Z][a-z]+(?:\s+[A-Z][a-z]+)*\b")
_TWO_CAP_PATTERN = re.compile(r"\b[A-Z][a-z]{2,}\s+[A-Z][a-z]{2,}\b")

# Known clinical terms that look like names but aren't — keep these out of the
# name redactor. Extend as needed in the deploying organization's config.
_NOT_NAMES = {
    "Blood Pressure", "Heart Rate", "Respiratory Rate", "Body Mass",
    "Chief Complaint", "Past Medical", "History Of", "Present Illness",
    "Social History", "Family History", "Review Of", "Physical Exam",
    "Mental Status", "General Appearance", "Vital Signs",
}


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class ScrubResult:
    text: str
    redactions: dict[str, int]

    def total(self) -> int:
        return sum(self.redactions.values())


def scrub(text: str, mode: str = "safe_harbor") -> ScrubResult:
    """Redact PHI from ``text``.

    Args:
        text: input string (may contain PHI).
        mode: "safe_harbor" — full Safe Harbor scrubbing.
              "encounter_handle" — only used for hashing; raises if called for text.

    Returns:
        ScrubResult with redacted text and counts of each redaction category.
    """
    if mode == "encounter_handle":
        raise ValueError("Use encounter_handle() to compute a handle; scrub() does not apply in this mode.")
    if mode != "safe_harbor":
        raise ValueError(f"unknown scrub mode: {mode!r}")

    counts: dict[str, int] = {}
    out = text

    for label, pat in _PATTERNS:
        new_out, n = pat.subn(f"[REDACTED:{label}]", out)
        if n:
            counts[label] = counts.get(label, 0) + n
        out = new_out

    # Names: titled first
    new_out, n = _NAME_PATTERN.subn("[REDACTED:NAME]", out)
    if n:
        counts["NAME"] = counts.get("NAME", 0) + n
    out = new_out

    # Two-cap heuristic, with allowlist
    def _name_sub(m: re.Match[str]) -> str:
        if m.group(0) in _NOT_NAMES:
            return m.group(0)
        return "[REDACTED:NAME]"

    new_out, n_total = _TWO_CAP_PATTERN.subn(_name_sub, out)
    # Count only actual replacements
    actual = new_out.count("[REDACTED:NAME]") - out.count("[REDACTED:NAME]")
    if actual > 0:
        counts["NAME"] = counts.get("NAME", 0) + actual
    out = new_out

    return ScrubResult(text=out, redactions=counts)


def encounter_handle(*identifiers: str, salt: str | None = None) -> str:
    """Produce a stable, non-reversible handle for an encounter.

    Combines the supplied identifiers (e.g. encounter date+location+ID) under a
    keyed HMAC. The salt should be a per-deployment secret pulled from the
    environment; without one, falls back to a build-time constant which is
    documented as lower-assurance.

    The resulting hex digest is suitable for the ``encounter_handle`` field on
    audit events. It is deterministic for the same inputs+salt, and not
    reversible without the salt.
    """
    if not identifiers:
        raise ValueError("at least one identifier required")
    key = (salt or os.environ.get("AMBIENT_SCRIBE_HANDLE_SALT") or "ambient-scribe-default-salt").encode("utf-8")
    msg = "\u241f".join(identifiers).encode("utf-8")  # U+241F is a unit separator
    return hmac.new(key, msg, hashlib.sha256).hexdigest()


def transcript_hash(text: str) -> str:
    """SHA-256 of normalized transcript text."""
    normalized = "\n".join(line.strip() for line in text.splitlines()).strip()
    return hashlib.sha256(normalized.encode("utf-8")).hexdigest()


def assert_scrubbed(text: str, *, allow: Iterable[str] = ()) -> None:
    """Raise if ``text`` appears to contain unredacted PHI.

    Use as a final guard before writing to a non-clinical destination.
    ``allow`` lists redaction labels that are acceptable to find (e.g. the
    placeholder strings themselves).
    """
    result = scrub(text)
    leaked = {k: v for k, v in result.redactions.items() if k not in set(allow)}
    if leaked:
        raise PHILeakError(f"unredacted PHI detected: {leaked}")


class PHILeakError(RuntimeError):
    """Raised when PHI is detected in text bound for a non-clinical destination."""


# ---------------------------------------------------------------------------
# CLI for spot-checking
# ---------------------------------------------------------------------------

if __name__ == "__main__":  # pragma: no cover
    import sys
    src = sys.stdin.read()
    result = scrub(src)
    sys.stdout.write(result.text)
    sys.stderr.write(f"redactions: {result.redactions}\n")
