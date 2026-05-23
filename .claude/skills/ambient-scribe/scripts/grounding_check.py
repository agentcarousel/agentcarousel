"""
Grounding check for ambient-scribe.

Verifies that every clinical claim in a SOAP draft cites transcript turns that
exist and that plausibly support the claim. Failures are returned as structured
records so the calling skill can move them into Needs Clinician Verification
rather than silently dropping them.

This script does the deterministic part of grounding (citation existence, citation
coverage, and turn-text retrieval). The semantic part — *does the cited turn
actually support the claim* — is ultimately a model judgment; this script
prepares the verification prompts and exposes a hook (``verifier``) so the
calling agent can plug in a model call. A no-op default verifier is provided
that approves all extant citations; the deploying organization should replace it
with a real model-backed verifier in production.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Callable, Iterable

# Regex for citations like [T0014] or [T0014, T0017] or [T0014–T0016]
_CITE_BLOCK = re.compile(r"\[(T\d{4,}(?:\s*[,–-]\s*T?\d{4,})*)\]")
_TURN_ID = re.compile(r"T\d{4,}")
_TURN_LINE = re.compile(r"^(T\d{4,})\s+([A-Za-z][A-Za-z ]*)?:?\s*(.*)$")


@dataclass(frozen=True)
class Turn:
    id: str
    speaker: str | None
    text: str


@dataclass
class Claim:
    section: str            # "S" | "O" | "A" | "P"
    sentence: str           # full sentence including the citation block
    cited_turns: list[str]


@dataclass
class GroundingFailure:
    section: str
    sentence: str
    reason: str             # "missing_citation" | "orphan_citation" | "unsupported"
    cited_turns: list[str] = field(default_factory=list)
    missing_turns: list[str] = field(default_factory=list)
    verifier_note: str | None = None


@dataclass
class GroundingReport:
    claims: list[Claim]
    failures: list[GroundingFailure]

    @property
    def passed(self) -> bool:
        return not self.failures

    def summary_counts(self) -> dict[str, int]:
        out = {"missing_citation": 0, "orphan_citation": 0, "unsupported": 0}
        for f in self.failures:
            out[f.reason] = out.get(f.reason, 0) + 1
        return out


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------

def parse_transcript(text: str) -> dict[str, Turn]:
    """Parse a transcript with turn-prefixed lines into a dict of Turn by id."""
    turns: dict[str, Turn] = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        m = _TURN_LINE.match(line)
        if not m:
            continue
        turn_id, speaker, body = m.group(1), m.group(2), m.group(3)
        turns[turn_id] = Turn(id=turn_id, speaker=(speaker or None) and speaker.strip(), text=body.strip())
    return turns


def _expand_citation_block(block: str) -> list[str]:
    """Expand 'T0014, T0017' or 'T0014–T0016' to ['T0014','T0017'] or ['T0014','T0015','T0016']."""
    out: list[str] = []
    for piece in re.split(r"\s*,\s*", block):
        # range?
        rng = re.match(r"^(T\d{4,})\s*[–-]\s*T?(\d{4,})$", piece)
        if rng:
            start = int(rng.group(1)[1:])
            end = int(rng.group(2))
            width = max(4, len(rng.group(1)) - 1)
            for i in range(start, end + 1):
                out.append("T" + str(i).zfill(width))
        else:
            ids = _TURN_ID.findall(piece)
            out.extend(ids)
    # de-duplicate, preserve order
    seen: set[str] = set()
    uniq: list[str] = []
    for t in out:
        if t not in seen:
            uniq.append(t)
            seen.add(t)
    return uniq


def _split_into_section_sentences(soap_text: str) -> list[Claim]:
    """Pull S/O/A/P sentences out of a SOAP draft.

    Recognizes either 'Subjective:' / 'Objective:' / 'Assessment:' / 'Plan:' inline
    headers, or '### Subjective' / '### Objective' / etc. style headings.
    Sentences are the period/question-mark/exclamation-terminated spans.
    """
    section_map = {
        "subjective": "S",
        "objective": "O",
        "assessment": "A",
        "plan": "P",
    }
    pattern = re.compile(
        r"(?im)^(?:#{1,4}\s*)?(?:\*\*)?(subjective|objective|assessment|plan)(?:\*\*)?\s*:?\s*$"
    )
    inline_pattern = re.compile(
        r"(?i)\*\*(subjective|objective|assessment|plan)\s*:\*\*\s*"
    )

    # Build a flat sequence of (section_letter, body_text)
    sections: list[tuple[str, str]] = []

    # First try heading-style splits.
    matches = list(pattern.finditer(soap_text))
    if matches:
        for i, m in enumerate(matches):
            section = section_map[m.group(1).lower()]
            start = m.end()
            end = matches[i + 1].start() if i + 1 < len(matches) else len(soap_text)
            body = soap_text[start:end]
            sections.append((section, body))
    else:
        # Inline-style: split on **Subjective:** etc.
        cursor = 0
        cur_section: str | None = None
        for m in inline_pattern.finditer(soap_text):
            if cur_section:
                sections.append((cur_section, soap_text[cursor:m.start()]))
            cur_section = section_map[m.group(1).lower()]
            cursor = m.end()
        if cur_section:
            sections.append((cur_section, soap_text[cursor:]))

    claims: list[Claim] = []
    for section, body in sections:
        # Sentence split: period/question/exclam followed by whitespace, but NOT
        # if the next non-space token is a citation block — citations attach to
        # the preceding sentence per references/soap-format.md.
        for sent in re.split(r"(?<=[.!?])\s+(?!\[T\d)", body.strip()):
            s = sent.strip()
            if not s or s.startswith(("|", "-", "#")):  # tables, bullets, headings
                continue
            if _looks_like_boilerplate(s):
                continue
            cited: list[str] = []
            for cm in _CITE_BLOCK.finditer(s):
                cited.extend(_expand_citation_block(cm.group(1)))
            claims.append(Claim(section=section, sentence=s, cited_turns=cited))
    return claims


_BOILERPLATE_PHRASES = (
    "not addressed in encounter",
    "not performed",
    "not described in encounter",
    "the following history was obtained",
)


def _looks_like_boilerplate(s: str) -> bool:
    low = s.lower().strip(" .")
    return any(low == p or low.endswith(p) for p in _BOILERPLATE_PHRASES)


# ---------------------------------------------------------------------------
# Verification
# ---------------------------------------------------------------------------

# A verifier takes (claim_sentence, [Turn]) and returns (supported: bool, note: str|None).
Verifier = Callable[[str, list[Turn]], tuple[bool, str | None]]


def default_verifier(claim_sentence: str, turns: list[Turn]) -> tuple[bool, str | None]:
    """No-op verifier: approves whenever the cited turns exist and are non-empty.

    Replace with a model-backed verifier in production. The skill's calling
    agent can wrap a Claude messages call here and ask whether the cited turn(s)
    plausibly support the claim, returning (False, reason) on negative answers.
    """
    if not turns:
        return False, "no cited turns retrievable"
    if all(not t.text.strip() for t in turns):
        return False, "cited turns are empty"
    return True, None


# ---------------------------------------------------------------------------
# Top-level entry
# ---------------------------------------------------------------------------

def check(
    transcript_text: str,
    soap_draft: str,
    *,
    verifier: Verifier = default_verifier,
    sections_requiring_citations: Iterable[str] = ("S", "O", "A", "P"),
) -> GroundingReport:
    turns = parse_transcript(transcript_text)
    claims = _split_into_section_sentences(soap_draft)
    failures: list[GroundingFailure] = []
    required = set(sections_requiring_citations)

    for claim in claims:
        if claim.section not in required:
            continue

        # Coverage
        if not claim.cited_turns:
            failures.append(GroundingFailure(
                section=claim.section,
                sentence=claim.sentence,
                reason="missing_citation",
            ))
            continue

        # Existence
        missing = [tid for tid in claim.cited_turns if tid not in turns]
        if missing:
            failures.append(GroundingFailure(
                section=claim.section,
                sentence=claim.sentence,
                reason="orphan_citation",
                cited_turns=claim.cited_turns,
                missing_turns=missing,
            ))
            continue

        # Support (semantic)
        cited_turn_objs = [turns[tid] for tid in claim.cited_turns]
        supported, note = verifier(claim.sentence, cited_turn_objs)
        if not supported:
            failures.append(GroundingFailure(
                section=claim.section,
                sentence=claim.sentence,
                reason="unsupported",
                cited_turns=claim.cited_turns,
                verifier_note=note,
            ))

    return GroundingReport(claims=claims, failures=failures)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

if __name__ == "__main__":  # pragma: no cover
    import argparse
    import json
    import sys

    p = argparse.ArgumentParser(description="Grounding check for ambient-scribe drafts")
    p.add_argument("--transcript", required=True, help="Path to transcript .txt")
    p.add_argument("--draft", required=True, help="Path to SOAP draft .md")
    args = p.parse_args()

    with open(args.transcript, encoding="utf-8") as f:
        transcript_text = f.read()
    with open(args.draft, encoding="utf-8") as f:
        draft = f.read()

    report = check(transcript_text, draft)
    out = {
        "passed": report.passed,
        "claim_count": len(report.claims),
        "failure_count": len(report.failures),
        "failures": [
            {
                "section": f.section,
                "reason": f.reason,
                "sentence": f.sentence,
                "cited_turns": f.cited_turns,
                "missing_turns": f.missing_turns,
                "verifier_note": f.verifier_note,
            }
            for f in report.failures
        ],
        "summary": report.summary_counts(),
    }
    json.dump(out, sys.stdout, indent=2)
    sys.stdout.write("\n")
    sys.exit(0 if report.passed else 1)
