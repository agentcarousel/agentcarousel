#!/usr/bin/env python3
"""
generate-auditor-pdf.py — Build a human-readable auditor review package.

Usage (run from repo root):
  # Auto-discover last 5 bundle-matching runs from local run history:
  ./venv/bin/python3 scripts/generate-auditor-pdf.py \\
      --bundle fixtures/bundles/cmmc-assessor \\
      [--last 5]

  # Or supply run data manually:
  ./venv/bin/python3 scripts/generate-auditor-pdf.py \\
      --bundle fixtures/bundles/cmmc-assessor \\
      --runs-dir reports/carousel-runs

  # Or from exported evidence tarballs:
  ./venv/bin/python3 scripts/generate-auditor-pdf.py \\
      --bundle fixtures/bundles/cmmc-assessor \\
      --tarball reports/evidence-packs/cmmc-assessor/agentcarousel-evidence-*.tar.gz

Outputs a PDF (Chrome headless or weasyprint), falling back to print-ready HTML.
"""

import argparse
import copy
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import textwrap
import time
from datetime import datetime, timezone
from pathlib import Path

import yaml
from jinja2 import Environment, BaseLoader
from markdown import markdown as md_to_html
from pygments import highlight
from pygments.formatters import HtmlFormatter
from pygments.lexers import JsonLexer, YamlLexer, TextLexer, MarkdownLexer

# ── repo root is two levels up from this script ──────────────────────────────
REPO_ROOT = Path(__file__).resolve().parent.parent


# ─────────────────────────────────────────────────────────────────────────────
# Data loading
# ─────────────────────────────────────────────────────────────────────────────

def load_manifest(bundle_dir: Path) -> dict:
    path = bundle_dir / "bundle.manifest.json"
    return json.loads(path.read_text())


def load_fixture(manifest: dict) -> dict:
    """Return parsed YAML for the skill fixture referenced in the manifest."""
    skill = manifest.get("skill_or_agent", "")
    candidates = [
        REPO_ROOT / "fixtures" / "skills" / f"{skill}.yaml",
        REPO_ROOT / f"fixtures/skills/{skill}.yaml",
    ]
    for path in candidates:
        if path.exists():
            return yaml.safe_load(path.read_text())
    return {}


def load_judge_prompt(manifest: dict) -> str:
    skill = manifest.get("skill_or_agent", "")
    path = REPO_ROOT / "docs" / "judge-prompts" / f"{skill}-v1.md"
    if path.exists():
        return path.read_text()
    return ""


def load_golden_files(fixture: dict) -> list[dict]:
    """Return list of {path, content} for each unique golden_path referenced."""
    seen, results = set(), []
    for case in fixture.get("cases", []):
        cfg = case.get("evaluator_config", {})
        golden_path = cfg.get("golden_path", "")
        if golden_path and golden_path not in seen:
            seen.add(golden_path)
            abs_path = REPO_ROOT / golden_path
            if abs_path.exists():
                results.append({"path": golden_path, "content": abs_path.read_text()})
    return results


def load_runs(runs_dir: Path | None, run_tarballs: list[Path]) -> list[dict]:
    """Load carousel run data from a directory of JSON files and/or tarballs."""
    runs = []

    if runs_dir and runs_dir.is_dir():
        for p in sorted(runs_dir.glob("*.json")):
            try:
                runs.append(json.loads(p.read_text()))
            except Exception:
                pass

    for tarball in run_tarballs:
        if not tarball.exists():
            continue
        try:
            with tarfile.open(tarball) as tf:
                for member in tf.getmembers():
                    if member.name.endswith("run.json"):
                        f = tf.extractfile(member)
                        if f:
                            runs.append(json.loads(f.read()))
        except Exception:
            pass

    return runs


# ─────────────────────────────────────────────────────────────────────────────
# Auto-discovery: last N bundle-matching runs via the agentcarousel CLI
# ─────────────────────────────────────────────────────────────────────────────

AGC_CANDIDATES = [
    str(REPO_ROOT / "target" / "release" / "agentcarousel"),
    str(REPO_ROOT / "target" / "debug"   / "agentcarousel"),
    "agentcarousel",
]


def find_agc_bin() -> str | None:
    env_override = os.environ.get("AGENTCAROUSEL_BIN")
    if env_override:
        return env_override
    for candidate in AGC_CANDIDATES:
        if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
        result = subprocess.run(["which", candidate], capture_output=True, text=True)
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()
    return None


def _run_agc(agc: str, args: list[str], check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run([agc] + args, capture_output=True, text=True, check=check)


def fetch_bundle_runs(bundle_dir: Path, n: int = 5) -> list[dict]:
    """
    Discover the N most recent runs for this bundle from local history, export
    each as a tarball into a temp dir, and return the parsed run dicts.

    Steps:
      1. agentcarousel publish --dry-run --all-runs --limit N  → run_ids[]
      2. agentcarousel export <id>  → tarball  → run.json  for each id
    """
    import tempfile

    agc = find_agc_bin()
    if agc is None:
        print(
            "warning: agentcarousel binary not found; skipping auto-discovery.\n"
            "  Set AGENTCAROUSEL_BIN or build with `cargo build --release`.",
            file=sys.stderr,
        )
        return []

    # Step 1: get run IDs matching this bundle
    result = _run_agc(
        agc,
        [
            "publish", str(bundle_dir),
            "--dry-run", "--all-runs",
            "--limit", str(n),
            "--url", "https://placeholder.invalid",
        ],
        check=False,
    )
    if result.returncode != 0:
        print(
            f"warning: could not discover runs for bundle ({result.stderr.strip()[:120]})",
            file=sys.stderr,
        )
        return []

    try:
        payload  = json.loads(result.stdout)
        run_ids  = payload.get("run_ids", [])
    except Exception as exc:
        print(f"warning: could not parse publish --dry-run output: {exc}", file=sys.stderr)
        return []

    if not run_ids:
        print("warning: no bundle-matching runs found in local history.", file=sys.stderr)
        return []

    print(f"Found {len(run_ids)} run(s) for bundle; exporting…")

    # Step 2: export each run and read run.json from the tarball
    runs = []
    with tempfile.TemporaryDirectory(prefix="agc-auditor-") as tmpdir:
        for run_id in run_ids:
            tarball_path = Path(tmpdir) / f"agentcarousel-evidence-{run_id}.tar.gz"
            exp = _run_agc(
                agc,
                ["export", run_id, "--out", str(tarball_path)],
                check=False,
            )
            if exp.returncode != 0 or not tarball_path.exists():
                print(f"  warning: export failed for {run_id}: {exp.stderr.strip()[:80]}", file=sys.stderr)
                continue
            try:
                with tarfile.open(tarball_path) as tf:
                    for member in tf.getmembers():
                        if member.name.endswith("run.json"):
                            fh = tf.extractfile(member)
                            if fh:
                                runs.append(json.loads(fh.read()))
                                print(f"  loaded run {run_id}")
                                break
            except Exception as exc:
                print(f"  warning: could not read tarball for {run_id}: {exc}", file=sys.stderr)

    return runs


# ─────────────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────────────

def hl_json(text: str) -> str:
    return highlight(text, JsonLexer(), HtmlFormatter(nowrap=True))


def hl_yaml(text: str) -> str:
    return highlight(text, YamlLexer(), HtmlFormatter(nowrap=True))


def hl_text(text: str) -> str:
    return highlight(text, TextLexer(), HtmlFormatter(nowrap=True))


def md(text: str) -> str:
    return md_to_html(text, extensions=["tables", "fenced_code"])


def weight_bar(weight: float) -> str:
    pct = int(round(weight * 100))
    return (
        f'<span class="weight-bar" style="width:{pct * 2}px"></span>'
        f'<span class="weight-label">{pct}%</span>'
    )


def status_badge(status: str) -> str:
    cls = "pass" if status in ("passed", "Passed", "PASS") else "fail"
    return f'<span class="badge {cls}">{status.upper()}</span>'


def rubric_variance(runs: list[dict]) -> dict[str, float]:
    """
    Compute per-rubric-item score variance across runs.
    Returns {rubric_id: variance_pct} where variance_pct = (max-min)*100.
    """
    scores: dict[str, list[float]] = {}
    for run in runs:
        for case in run.get("cases", []):
            eval_scores = case.get("eval_scores") or {}
            for item in eval_scores.get("rubric_scores", []):
                rid = item.get("rubric_id", "")
                score = item.get("score")
                if rid and score is not None:
                    scores.setdefault(rid, []).append(float(score))
    return {
        rid: round((max(v) - min(v)) * 100, 1)
        for rid, v in scores.items()
        if len(v) > 1
    }


def _render_value(v) -> str:
    if isinstance(v, (dict, list)):
        return json.dumps(v, indent=2)
    return str(v or "")


def build_conversation(fixture_messages: list[dict], trace_steps: list[dict]) -> list[dict]:
    """
    Interleave fixture input messages with intermediate LLM turns so the auditor
    sees the full back-and-forth thread in chronological order.

    The fixture defines every user turn (and any pre-seeded assistant turns) in
    input.messages[]. When the agent produces an intermediate response and then
    the conversation continues (multi-turn), those user follow-ups are already in
    the messages list. We reconstruct the sequence as:

      user_msg[0]  →  [tool calls if any]  →  llm intermediate[0]  →
      user_msg[1]  →  ...  →  final_output

    Returns a list of turn dicts: {role, content, kind}
    where role ∈ {"user","assistant"} and kind ∈ {"input","intermediate","final","tool_call","tool_result"}.
    """
    turns: list[dict] = []

    # Separate fixture messages by role
    fixture_user_turns    = [m for m in fixture_messages if m.get("role") == "user"]
    fixture_assist_turns  = [m for m in fixture_messages if m.get("role") == "assistant"]

    # Separate trace steps by kind
    llm_steps    = [s for s in trace_steps if s.get("kind") == "llm_call"]
    tool_steps   = [s for s in trace_steps if s.get("kind") in ("tool_call", "tool_result")]

    # If there's only one user message and no intermediate LLM calls, it's single-turn
    if len(fixture_user_turns) <= 1 and len(llm_steps) <= 1 and not fixture_assist_turns:
        if fixture_user_turns:
            turns.append({"role": "user", "content": fixture_user_turns[0].get("content", ""), "kind": "input"})
        for step in tool_steps:
            if step.get("kind") == "tool_call":
                turns.append({
                    "role": "tool",
                    "content": f"→ {step.get('tool', '')}\n{_render_value(step.get('args'))}",
                    "kind": "tool_call",
                })
            else:
                turns.append({
                    "role": "tool",
                    "content": f"← {step.get('tool', '')}\n{_render_value(step.get('result'))}",
                    "kind": "tool_result",
                })
        return turns  # final_output added by caller

    # Multi-turn: interleave user messages, pre-seeded assistant turns, tool steps, and LLM steps
    # Reconstruct in index order using step.index as the clock
    all_events: list[tuple[int, dict]] = []

    # Assign synthetic indices to fixture messages (they precede trace events)
    for i, msg in enumerate(fixture_messages):
        all_events.append((-1000 + i, {"_type": "fixture_msg", **msg}))

    for step in trace_steps:
        idx = step.get("index", 9999)
        all_events.append((idx, {"_type": "trace_step", **step}))

    all_events.sort(key=lambda x: x[0])

    seen_fixture_user = 0  # how many fixture user messages we've emitted
    for _, event in all_events:
        etype = event.get("_type")
        if etype == "fixture_msg":
            role = event.get("role", "user")
            content = event.get("content", "")
            if role == "user":
                turns.append({"role": "user", "content": content,
                               "kind": "input" if seen_fixture_user == 0 else "user_followup"})
                seen_fixture_user += 1
            elif role == "assistant":
                turns.append({"role": "assistant", "content": content, "kind": "seeded"})
        elif etype == "trace_step":
            kind = event.get("kind", "")
            if kind == "tool_call":
                turns.append({
                    "role": "tool",
                    "content": f"→ {event.get('tool', '')}\n{_render_value(event.get('args'))}",
                    "kind": "tool_call",
                })
            elif kind == "tool_result":
                turns.append({
                    "role": "tool",
                    "content": f"← {event.get('tool', '')}\n{_render_value(event.get('result'))}",
                    "kind": "tool_result",
                })
            elif kind == "llm_call":
                result_val = event.get("result")
                if result_val:
                    text = result_val if isinstance(result_val, str) else _render_value(result_val)
                    turns.append({"role": "assistant", "content": text, "kind": "intermediate"})

    return turns


def build_transcripts(runs: list[dict], fixture_cases_by_id: dict) -> list[dict]:
    """
    Return one entry per case_id with the full conversation thread and rubric
    scores from every run, for the Transcripts section.

    Shape: [{case_id, runs: [{run_index, run_id, status, effectiveness,
                               conversation, final_output, rubric_scores,
                               judge_rationale, latency_ms, redacted}]}]

    conversation is a list of turn dicts: {role, content, kind}
    final_output is the agent's last response (appended as kind="final").
    """
    case_order: list[str] = []
    case_map: dict[str, list[dict]] = {}

    for run_idx, run in enumerate(runs):
        run_id = run.get("id", f"run-{run_idx + 1}")
        for case in run.get("cases", []):
            cid = case.get("case_id", "")
            if not cid:
                continue
            if cid not in case_map:
                case_order.append(cid)
                case_map[cid] = []

            trace   = case.get("trace") or {}
            eval_sc = case.get("eval_scores") or {}
            metrics = case.get("metrics") or {}

            fixture_case     = fixture_cases_by_id.get(cid, {})
            fixture_messages = fixture_case.get("input", {}).get("messages", [])
            trace_steps      = trace.get("steps", [])
            final_output     = trace.get("final_output") or ""
            redacted         = trace.get("redacted", False)

            conversation = build_conversation(fixture_messages, trace_steps)
            if final_output and not redacted:
                conversation.append({"role": "assistant", "content": final_output, "kind": "final"})
            elif redacted:
                conversation.append({"role": "assistant", "content": "[output redacted]", "kind": "final"})

            case_map[cid].append({
                "run_index":       run_idx + 1,
                "run_id":          run_id,
                "status":          case.get("status", "unknown"),
                "effectiveness":   eval_sc.get("effectiveness_score"),
                "conversation":    conversation,
                "rubric_scores":   eval_sc.get("rubric_scores", []),
                "judge_rationale": eval_sc.get("judge_rationale") or "",
                "latency_ms":      metrics.get("total_latency_ms", 0),
                "redacted":        redacted,
            })

    return [{"case_id": cid, "runs": case_map[cid]} for cid in case_order]


# ─────────────────────────────────────────────────────────────────────────────
# HTML template
# ─────────────────────────────────────────────────────────────────────────────

PYGMENTS_CSS = HtmlFormatter().get_style_defs(".highlight")

HTML_TEMPLATE = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"/>
<title>Auditor Review Package — {{ manifest.bundle_id }}@{{ manifest.bundle_version }}</title>
<style>
/* ── Reset & base ── */
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: "Georgia", "Times New Roman", serif; font-size: 11pt;
       line-height: 1.6; color: #1a1a1a; background: #fff; }
h1 { font-size: 22pt; margin-bottom: 0.3em; }
h2 { font-size: 15pt; margin: 2em 0 0.5em; border-bottom: 2px solid #2c5282; padding-bottom: 0.2em; color: #2c5282; }
h3 { font-size: 12pt; margin: 1.4em 0 0.4em; color: #2d3748; }
h4 { font-size: 11pt; margin: 1em 0 0.3em; color: #4a5568; font-style: italic; }
p  { margin: 0.5em 0; }
ul, ol { margin: 0.5em 0 0.5em 1.5em; }
li { margin: 0.2em 0; }
a  { color: #2b6cb0; }
table { border-collapse: collapse; width: 100%; margin: 0.8em 0; font-size: 10pt; }
th { background: #2c5282; color: #fff; padding: 6px 10px; text-align: left; }
td { padding: 5px 10px; border-bottom: 1px solid #e2e8f0; vertical-align: top; }
tr:nth-child(even) td { background: #f7fafc; }
blockquote { border-left: 4px solid #bee3f8; padding: 0.5em 1em; margin: 0.8em 0;
             background: #ebf8ff; font-style: italic; }

/* ── Code blocks ── */
pre  { background: #f7fafc; border: 1px solid #e2e8f0; border-radius: 4px;
       padding: 0.8em 1em; font-size: 8.5pt; font-family: "Menlo","Consolas",monospace;
       white-space: pre-wrap; word-break: break-all; overflow-wrap: anywhere; margin: 0.6em 0; }
code { font-family: "Menlo","Consolas",monospace; font-size: 9pt;
       background: #edf2f7; padding: 1px 4px; border-radius: 3px; }
.highlight { background: #f7fafc !important; border: 1px solid #e2e8f0;
             border-radius: 4px; padding: 0.6em 0.8em; margin: 0.6em 0; font-size: 8.5pt; }
{{ pygments_css }}

/* ── Badges ── */
.badge { display: inline-block; padding: 2px 8px; border-radius: 10px;
         font-size: 8pt; font-weight: bold; font-family: sans-serif; }
.badge.pass { background: #c6f6d5; color: #22543d; }
.badge.fail { background: #fed7d7; color: #742a2a; }
.track-badge { background: #bee3f8; color: #2a4365; font-family: sans-serif;
               font-size: 9pt; padding: 2px 8px; border-radius: 10px; }
.risk-high { background: #fed7d7; color: #742a2a; font-family: sans-serif;
             font-size: 9pt; padding: 2px 8px; border-radius: 10px; }

/* ── Weight bar ── */
.weight-bar { display: inline-block; height: 10px; background: #4299e1;
              border-radius: 3px; vertical-align: middle; margin-right: 4px; }
.weight-label { font-family: sans-serif; font-size: 8.5pt; color: #4a5568; }

/* ── Layout ── */
.page { max-width: 780px; margin: 0 auto; padding: 2em 3em; }
.cover { text-align: center; padding: 6em 2em; border-bottom: 3px solid #2c5282; margin-bottom: 3em; }
.cover .subtitle { font-size: 13pt; color: #4a5568; margin-top: 0.4em; }
.cover .meta { margin-top: 2em; font-family: sans-serif; font-size: 10pt; color: #4a5568; }
.cover .meta td { border: none; background: none; padding: 3px 12px; }
.cover .meta th { background: none; color: #2d3748; padding: 3px 12px; font-weight: normal; }
.toc { background: #f7fafc; border: 1px solid #e2e8f0; border-radius: 6px;
       padding: 1.2em 1.5em; margin-bottom: 2em; font-family: sans-serif; }
.toc h2 { font-size: 12pt; margin: 0 0 0.6em; border: none; }
.toc ol { margin: 0 0 0 1.2em; font-size: 10pt; }
.toc li { margin: 0.2em 0; }
.section { margin-bottom: 2.5em; }
.case-block { border: 1px solid #e2e8f0; border-radius: 6px; padding: 1em 1.2em;
              margin: 1em 0; background: #fafafa; }
.rubric-item { border-left: 3px solid #4299e1; padding: 0.5em 0.8em;
               margin: 0.5em 0; background: #ebf8ff; }
.rubric-item.high { border-color: #48bb78; }
.input-box { background: #fffdf0; border: 1px solid #f6e05e; border-radius: 4px;
             padding: 0.8em 1em; margin: 0.6em 0; }
.expected-box { background: #f0fff4; border: 1px solid #9ae6b4; border-radius: 4px;
                padding: 0.8em 1em; margin: 0.6em 0; }
.signing-box { background: #fffaf0; border: 2px solid #ed8936; border-radius: 6px;
               padding: 1.2em 1.5em; margin: 1em 0; }
.alert { background: #fff5f5; border: 1px solid #fc8181; border-radius: 4px;
         padding: 0.6em 1em; margin: 0.6em 0; font-family: sans-serif; font-size: 10pt; }
.variance-ok  { color: #22543d; font-weight: bold; }
.variance-warn { color: #744210; font-weight: bold; }
.variance-fail { color: #742a2a; font-weight: bold; }
.divider { border: none; border-top: 1px solid #e2e8f0; margin: 1.5em 0; }

/* ── Transcripts ── */
.transcript-run { border: 1px solid #e2e8f0; border-radius: 6px; margin: 0.8em 0; }
.transcript-run-header { display: flex; align-items: center; gap: 0.6em;
                          background: #edf2f7; padding: 0.5em 0.8em;
                          border-radius: 6px 6px 0 0; font-family: sans-serif; font-size: 9.5pt; }
.transcript-run-header .run-id { font-family: monospace; color: #4a5568; font-size: 8.5pt; }
/* conversation thread */
.convo { padding: 0.6em 0.8em; border-top: 1px solid #e2e8f0; }
.turn { margin: 0.5em 0; }
.turn-label { font-family: sans-serif; font-size: 8pt; font-weight: bold;
              text-transform: uppercase; letter-spacing: 0.05em; margin-bottom: 0.2em; }
.turn-label.user     { color: #2b6cb0; }
.turn-label.followup { color: #2b6cb0; }
.turn-label.agent    { color: #276749; }
.turn-label.tool     { color: #744210; }
.turn-label.seeded   { color: #553c9a; }
.turn-body { white-space: pre-wrap; word-break: break-word; font-size: 9pt;
             line-height: 1.55; padding: 0.4em 0.6em; border-radius: 4px; }
.turn-body.user-body     { background: #ebf8ff; border-left: 3px solid #4299e1; }
.turn-body.followup-body { background: #ebf8ff; border-left: 3px solid #4299e1; }
.turn-body.agent-body    { background: #f0fff4; border-left: 3px solid #48bb78; font-family: Georgia, serif; }
.turn-body.final-body    { background: #f0fff4; border-left: 3px solid #276749; font-family: Georgia, serif; }
.turn-body.intermediate-body { background: #fffff0; border-left: 3px solid #d69e2e; font-family: Georgia, serif; font-style: italic; }
.turn-body.tool-body     { background: #faf5ff; border-left: 3px solid #9f7aea;
                            font-family: monospace; font-size: 8pt; }
.turn-body.seeded-body   { background: #fff5f7; border-left: 3px solid #ed64a6; font-style: italic; }
.turn-body.redacted-body { background: #f7fafc; color: #a0aec0; font-style: italic; }
.rubric-score-table { width: 100%; margin: 0.4em 0; font-size: 8.5pt; }
.rubric-score-table th { background: #4a5568; }
.score-pass { color: #22543d; font-weight: bold; }
.score-fail { color: #742a2a; font-weight: bold; }
.judge-rationale { background: #fffaf0; border-left: 3px solid #ed8936;
                   padding: 0.4em 0.8em; margin-top: 0.5em; font-size: 9pt;
                   font-style: italic; }

/* ── Print ── */
@media print {
  body { font-size: 10pt; }
  .cover { page-break-after: always; }
  .toc   { page-break-after: always; }
  h2     { page-break-before: always; }
  h2:first-of-type { page-break-before: avoid; }
  .case-block { page-break-inside: avoid; }
  pre, .highlight { page-break-inside: avoid; font-size: 8pt; }
  a { color: inherit; text-decoration: none; }
  @page { margin: 2cm 2.5cm; }
}
</style>
</head>
<body>
<div class="page">

<!-- ═══════════════════════════════════════════════════════ COVER ═══ -->
<div class="cover">
  <h1>Auditor Review Package</h1>
  <div class="subtitle">{{ manifest.bundle_id }}  ·  v{{ manifest.bundle_version }}</div>
  <table class="meta" style="margin: 2em auto; display: inline-table;">
    <tr><th>Generated</th><td>{{ generated_at }}</td></tr>
    <tr><th>Domain</th><td>{{ manifest.get("domain", "—") }}</td></tr>
    <tr><th>Certification Track</th><td><span class="track-badge">{{ manifest.get("certification_track","—") }}</span></td></tr>
    <tr><th>Risk Tier</th><td><span class="risk-high">{{ manifest.get("risk_tier","—") }}</span></td></tr>
    <tr><th>Data Handling</th><td>{{ manifest.get("data_handling","—") }}</td></tr>
    <tr><th>Policy Version</th><td>{{ manifest.get("policy_version","—") }}</td></tr>
    <tr><th>Owners</th><td>{{ manifest.get("owners", []) | join(", ") }}</td></tr>
  </table>
  <p style="font-family:sans-serif; font-size:9pt; color:#718096; margin-top:2em;">
    CONFIDENTIAL — For domain auditor review only. Contains synthetic evaluation data.
  </p>
</div>

<!-- ═══════════════════════════════════════════════════════ TOC ═══ -->
<div class="toc">
  <h2>Contents</h2>
  <ol>
    <li><a href="#sec-instructions">Auditor Instructions</a></li>
    <li><a href="#sec-manifest">Bundle Manifest</a></li>
    <li><a href="#sec-scenarios">Test Scenarios &amp; Rubric</a> ({{ fixture.get("cases", []) | length }} cases)</li>
    {% if judge_prompt %}<li><a href="#sec-judge">Judge Scoring Guide</a></li>{% endif %}
    {% if golden_files %}<li><a href="#sec-golden">Golden Reference Output</a></li>{% endif %}
    {% if runs %}<li><a href="#sec-runs">Carousel Run Results</a> ({{ runs | length }} runs)</li>{% endif %}
    <li><a href="#sec-signing">Signing Instructions</a></li>
  </ol>
</div>

<!-- ═════════════════════════════════════════════ 1. INSTRUCTIONS ═══ -->
<div class="section" id="sec-instructions">
<h2>1 — Auditor Instructions</h2>

<p>You are the <strong>Domain Auditor</strong> for this bundle. Your role is to assess whether the
fixture scenarios and rubric items correctly represent quality for the stated skill domain, and
whether the carousel run results demonstrate stable, trustworthy behavior. You are <em>not</em>
the fixture author and must review independently.</p>

<h3>What to review</h3>
<ol>
  <li><strong>Scenario realism</strong> — Are the test inputs realistic for practitioners in this domain?
      Would a real user encounter these situations?</li>
  <li><strong>Rubric correctness</strong> — Do the rubric items and their weights correctly prioritize
      what matters in this domain? Are the <code>auto_check</code> regex patterns sound, or do they
      accept outputs that are technically wrong?</li>
  <li><strong>Factual accuracy</strong> — Are domain-specific facts correct (standards references,
      practice IDs, regulatory clauses, scoring methodologies)?</li>
  <li><strong>Failure-mode coverage</strong> — Does the bundle test realistic failure conditions,
      not just happy paths?</li>
  {% if runs %}
  <li><strong>Run stability</strong> — Are the {{ runs | length }} carousel run results consistent?
      Rubric variance must be &lt;10% across runs for advancement to Stable.</li>
  {% endif %}
</ol>

<h3>What to return</h3>
<ul>
  <li>Written findings (any rubric items you would change and why)</li>
  <li>A signed attestation of the evidence pack using your minisign private key (see §{{ "7" if runs else "6" }})</li>
  <li>Any open items that must be resolved before Trusted status</li>
</ul>

{% if not manifest.get("owners") or manifest.get("owners") == ["@agentcarousel"] %}
<div class="alert">⚠️  <strong>Pre-flight check:</strong> The <code>owners</code> field in the manifest contains only
<code>@agentcarousel</code>. A named individual GitHub handle is required before Trusted attestation.</div>
{% endif %}
</div>

<!-- ═══════════════════════════════════════════════════ 2. MANIFEST ═══ -->
<div class="section" id="sec-manifest">
<h2>2 — Bundle Manifest</h2>
<p style="font-family:sans-serif; font-size:9pt; color:#718096;">
  Source: <code>fixtures/bundles/{{ manifest.get("skill_or_agent","") }}/bundle.manifest.json</code>
</p>
<div class="highlight"><pre>{{ manifest_json_hl }}</pre></div>
</div>

<!-- ═══════════════════════════════════════════════ 3. SCENARIOS ═══ -->
<div class="section" id="sec-scenarios">
<h2>3 — Test Scenarios &amp; Rubric</h2>
<p>{{ fixture.get("cases", []) | length }} cases total.
   {{ cert_cases | length }} are certification cases.
   Each case lists: input, expected output assertions, and rubric with weights.</p>

{% for case in fixture.get("cases", []) %}
{% set is_cert = case.id in cert_case_ids %}
<div class="case-block">
  <h3>
    {{ loop.index }}. <code>{{ case.id }}</code>
    {% if is_cert %}<span class="badge pass" style="margin-left:8px">CERTIFICATION</span>{% endif %}
    {% for tag in (case.get("tags") or []) %}
      <span class="track-badge" style="margin-left:4px">{{ tag }}</span>
    {% endfor %}
  </h3>

  {% if case.get("description") %}
  <p>{{ case.description | trim }}</p>
  {% endif %}

  <!-- Input -->
  <h4>Input</h4>
  <div class="input-box">
    {% for msg in (case.get("input", {}).get("messages", [])) %}
    <p><strong style="font-family:sans-serif;font-size:9pt">{{ msg.role | upper }}</strong></p>
    <pre>{{ msg.content | trim }}</pre>
    {% endfor %}
  </div>

  <!-- Expected output assertions -->
  {% if case.get("expected", {}).get("output") %}
  <h4>Expected output assertions</h4>
  <div class="expected-box">
    <table>
      <thead><tr><th>Kind</th><th>Pattern / Value</th></tr></thead>
      <tbody>
      {% for check in case.expected.output %}
        <tr>
          <td><code>{{ check.kind }}</code></td>
          <td><code>{{ check.value | e }}</code></td>
        </tr>
      {% endfor %}
      </tbody>
    </table>
  </div>
  {% endif %}

  <!-- Rubric -->
  {% if case.get("expected", {}).get("rubric") %}
  <h4>Rubric</h4>
  {% for item in case.expected.rubric %}
  <div class="rubric-item {% if item.weight >= 0.35 %}high{% endif %}">
    <strong>{{ item.id }}</strong> — {{ weight_bar(item.weight) }}<br/>
    <p style="margin-top:0.3em">{{ item.description | trim }}</p>
    {% if item.get("auto_check") %}
    <p style="font-family:sans-serif;font-size:9pt;color:#4a5568;margin-top:0.3em">
      Auto-check (<code>{{ item.auto_check.kind }}</code>):
      <code>{{ item.auto_check.value | e }}</code>
    </p>
    {% endif %}
  </div>
  {% endfor %}
  {% endif %}

  <!-- Evaluator config note -->
  {% if case.get("evaluator_config") %}
  <p style="font-family:sans-serif;font-size:9pt;color:#718096;margin-top:0.6em">
    Evaluator: <code>{{ case.evaluator_config.evaluator }}</code>
    {% if case.evaluator_config.get("golden_path") %}
     · golden threshold: {{ case.evaluator_config.get("golden_threshold","—") }}
    {% endif %}
  </p>
  {% endif %}
</div>
{% endfor %}
</div>

<!-- ════════════════════════════════════════ 4. JUDGE PROMPT ═══ -->
{% if judge_prompt %}
<div class="section" id="sec-judge">
<h2>4 — Judge Scoring Guide</h2>
<p style="font-family:sans-serif; font-size:9pt; color:#718096;">
  Source: <code>docs/judge-prompts/{{ manifest.get("skill_or_agent","") }}-v1.md</code>
</p>
{{ judge_prompt_html }}
</div>
{% endif %}

<!-- ════════════════════════════════════════ 5. GOLDEN FILES ═══ -->
{% if golden_files %}
<div class="section" id="sec-golden">
<h2>5 — Golden Reference Output</h2>
<p>The golden evaluator scores responses against these reference outputs.
   Threshold is noted per-case in §3.</p>
{% for gf in golden_files %}
<h3><code>{{ gf.path }}</code></h3>
{{ gf.content_html }}
{% endfor %}
</div>
{% endif %}

<!-- ════════════════════════════════════════ 6. RUN RESULTS ═══ -->
{% if runs %}
{% set section_num = 6 if (judge_prompt or golden_files) else 4 %}
<div class="section" id="sec-runs">
<h2>{{ section_num }} — Carousel Run Results</h2>
<p>{{ runs | length }} eval run(s) provided. Rubric variance must be &lt;10% across all runs
   for Candidate → Stable advancement.</p>

<!-- Run summary table -->
<table>
  <thead>
    <tr>
      <th>#</th><th>Run ID</th><th>Started</th>
      <th>Pass rate</th><th>Mean effectiveness</th><th>Passed / Total</th>
    </tr>
  </thead>
  <tbody>
  {% for run in runs %}
  {% set summ = run.get("summary", {}) %}
  <tr>
    <td>{{ loop.index }}</td>
    <td style="font-family:monospace;font-size:9pt">{{ run.get("id","—") }}</td>
    <td style="font-family:sans-serif;font-size:9pt">{{ run.get("started_at","—")[:19] | replace("T"," ") }}</td>
    <td>{{ "%.0f%%" | format((summ.get("pass_rate",0) or 0) * 100) }}</td>
    <td>{{ "%.2f" | format(summ.get("mean_effectiveness_score") or 0) }}</td>
    <td>{{ summ.get("passed","—") }} / {{ summ.get("total","—") }}</td>
  </tr>
  {% endfor %}
  </tbody>
</table>

<!-- Per-case breakdown across runs -->
<h3>Per-case status across runs</h3>
{% set all_case_ids = [] %}
{% for run in runs %}
  {% for case in run.get("cases", []) %}
    {% if case.case_id not in all_case_ids %}{% set _ = all_case_ids.append(case.case_id) %}{% endif %}
  {% endfor %}
{% endfor %}
<table>
  <thead>
    <tr>
      <th>Case</th>
      {% for i in range(runs | length) %}<th>Run {{ i+1 }}</th>{% endfor %}
    </tr>
  </thead>
  <tbody>
  {% for cid in all_case_ids | sort %}
  <tr>
    <td style="font-family:monospace;font-size:9pt">{{ cid }}</td>
    {% for run in runs %}
      {% set case_match = run.get("cases", []) | selectattr("case_id","eq",cid) | list %}
      {% if case_match %}
        <td>{{ status_badge(case_match[0].get("status","?")) }}</td>
      {% else %}
        <td style="color:#a0aec0">—</td>
      {% endif %}
    {% endfor %}
  </tr>
  {% endfor %}
  </tbody>
</table>

<!-- Rubric variance -->
{% if variance %}
<h3>Rubric score variance across runs</h3>
<p style="font-family:sans-serif;font-size:9pt">
  Variance = max(score) − min(score) across {{ runs | length }} runs, expressed as percentage points.
  Must be &lt;10 pp for all certification rubric items to advance to Stable.
</p>
<table>
  <thead><tr><th>Rubric item</th><th>Variance (pp)</th><th>Status</th></tr></thead>
  <tbody>
  {% for rid, v in variance.items() | sort %}
  <tr>
    <td><code>{{ rid }}</code></td>
    <td>{{ v }}</td>
    <td>
      {% if v < 5 %}<span class="variance-ok">✓ stable</span>
      {% elif v < 10 %}<span class="variance-warn">⚠ borderline</span>
      {% else %}<span class="variance-fail">✗ exceeds 10 pp</span>{% endif %}
    </td>
  </tr>
  {% endfor %}
  </tbody>
</table>
{% endif %}
</div>
{% endif %}

<!-- ════════════════════════════════════════ 7. TRANSCRIPTS ═══ -->
{% if transcripts %}
{% set trans_sec = section_num + 1 %}
<div class="section" id="sec-transcripts">
<h2>{{ trans_sec }} — Run Transcripts</h2>
<p>Agent output and rubric scores for every case across all {{ runs | length }} run(s).
   This is the primary material for domain quality review.</p>

{% for entry in transcripts %}
{% set case_fixture = fixture.get("cases", []) | selectattr("id", "eq", entry.case_id) | list %}
{% set case_desc = case_fixture[0].description | trim if case_fixture else "" %}
<h3 style="margin-top:1.8em"><code>{{ entry.case_id }}</code></h3>
{% if case_desc %}<p style="font-size:9.5pt;color:#4a5568">{{ case_desc[:200] }}{% if case_desc|length > 200 %}…{% endif %}</p>{% endif %}

{% for r in entry.runs %}
<div class="transcript-run">
  <div class="transcript-run-header">
    <strong>Run {{ r.run_index }}</strong>
    {{ status_badge(r.status) }}
    {% if r.effectiveness is not none %}
      <span style="color:#4a5568">effectiveness: <strong>{{ "%.2f" | format(r.effectiveness) }}</strong></span>
    {% endif %}
    {% if r.latency_ms %}
      <span style="color:#718096;font-size:8.5pt">{{ r.latency_ms }}ms</span>
    {% endif %}
    <span class="run-id">{{ r.run_id }}</span>
  </div>

  <!-- Conversation thread -->
  <div class="convo">
  {% if r.conversation %}
    {% for turn in r.conversation %}
    <div class="turn">
      {% if turn.kind == "input" %}
        <div class="turn-label user">User</div>
        <div class="turn-body user-body">{{ turn.content | e }}</div>
      {% elif turn.kind == "user_followup" %}
        <div class="turn-label followup">User (follow-up)</div>
        <div class="turn-body followup-body">{{ turn.content | e }}</div>
      {% elif turn.kind == "seeded" %}
        <div class="turn-label seeded">Assistant (pre-seeded in fixture)</div>
        <div class="turn-body seeded-body">{{ turn.content | e }}</div>
      {% elif turn.kind == "intermediate" %}
        <div class="turn-label agent">Agent (intermediate response)</div>
        <div class="turn-body intermediate-body">{{ turn.content | e }}</div>
      {% elif turn.kind == "final" %}
        <div class="turn-label agent">Agent (final response)</div>
        <div class="turn-body {{ 'redacted-body' if r.redacted else 'final-body' }}">{{ turn.content | e }}</div>
      {% elif turn.kind in ("tool_call", "tool_result") %}
        <div class="turn-label tool">{{ "Tool call" if turn.kind == "tool_call" else "Tool result" }}</div>
        <div class="turn-body tool-body">{{ turn.content | e }}</div>
      {% endif %}
    </div>
    {% endfor %}
  {% else %}
    <div style="color:#a0aec0;font-style:italic;font-size:9pt">[no conversation recorded]</div>
  {% endif %}
  </div>

  <!-- Rubric scores -->
  {% if r.rubric_scores %}
  <div style="padding:0.6em 0.8em; border-top:1px solid #e2e8f0;">
    <table class="rubric-score-table">
      <thead><tr><th>Rubric item</th><th>Score</th><th>Weight</th><th>Rationale</th></tr></thead>
      <tbody>
      {% for rs in r.rubric_scores %}
      <tr>
        <td><code>{{ rs.rubric_id }}</code></td>
        <td class="{{ 'score-pass' if rs.score >= 0.7 else 'score-fail' }}">
          {{ "%.2f" | format(rs.score) }}
        </td>
        <td style="color:#718096">{{ "%.0f%%" | format(rs.weight * 100) }}</td>
        <td style="font-size:8.5pt;color:#4a5568">{{ rs.rationale or "—" }}</td>
      </tr>
      {% endfor %}
      </tbody>
    </table>
    {% if r.judge_rationale %}
    <div class="judge-rationale"><strong>Judge overall:</strong> {{ r.judge_rationale }}</div>
    {% endif %}
  </div>
  {% endif %}

</div><!-- .transcript-run -->
{% endfor %}
{% endfor %}

</div>
{% endif %}

<!-- ══════════════════════════════════════ SIGNING INSTRUCTIONS ═══ -->
{% set sign_sec = (trans_sec + 1) if transcripts else ((section_num + 1) if runs else (section_num)) %}
<div class="section" id="sec-signing">
<h2>{{ sign_sec if sign_sec is defined else "—" }} — Signing Instructions</h2>

<div class="signing-box">
<h3>Step 1 — Generate your minisign key (once only)</h3>
<pre>minisign -G -p auditor.pub -s auditor.key</pre>
<p>Share <code>auditor.pub</code> with the Operator. Keep <code>auditor.key</code> private.</p>

<h3>Step 2 — Sign the evidence pack</h3>
<pre>minisign -S -s auditor.key \\
  -m agentcarousel-evidence-&lt;run-id&gt;.tar.gz \\
  -x {{ manifest.get("skill_or_agent","bundle") }}-attestation.minisig \\
  -c "Auditor: &lt;Your Name&gt; | Bundle: {{ manifest.bundle_id }}@{{ manifest.bundle_version }} | Date: $(date -u +%Y-%m-%d)"</pre>

<h3>Step 3 — Send to the Operator</h3>
<ul>
  <li><code>auditor.pub</code> — your minisign public key</li>
  <li><code>{{ manifest.get("skill_or_agent","bundle") }}-attestation.minisig</code> — the signature file</li>
  <li>Written findings document (any rubric changes, open items, or approval statement)</li>
</ul>

<h3>What the Operator will verify</h3>
<pre>agentcarousel trust-check {{ manifest.get("skill_or_agent","") }}@{{ manifest.bundle_version }} \\
  --attestation {{ manifest.get("skill_or_agent","bundle") }}-attestation.minisig \\
  --minisign-pubkey auditor.pub</pre>

<h3>Registry trust-state payload (Operator submits after verification)</h3>
<div class="highlight"><pre>{{ trust_state_example }}</pre></div>
</div>

</div>
<!-- ════════════════════════════════════════════════════════ END ═══ -->
<hr class="divider"/>
<p style="font-family:sans-serif;font-size:8pt;color:#a0aec0;text-align:center;margin-top:2em;">
  Generated by <code>scripts/generate-auditor-pdf.py</code> on {{ generated_at }} ·
  agentcarousel/{{ manifest.bundle_id }}@{{ manifest.bundle_version }}
</p>

</div><!-- .page -->
</body>
</html>
"""


# ─────────────────────────────────────────────────────────────────────────────
# PDF / HTML output
# ─────────────────────────────────────────────────────────────────────────────

CHROME_CANDIDATES = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "google-chrome",
    "chromium",
    "chromium-browser",
]


def _chrome_bin() -> str | None:
    for candidate in CHROME_CANDIDATES:
        if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
        result = subprocess.run(["which", candidate], capture_output=True, text=True)
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()
    return None


def try_generate_pdf(html_path: Path, pdf_path: Path) -> bool:
    """
    Attempt PDF conversion using available backends, in preference order:
      1. Chrome / Chromium headless  (no system lib deps on macOS)
      2. weasyprint Python package   (needs libgobject on Linux/Mac)
    Returns True if PDF was written successfully.
    """
    # ── Backend 1: Chrome headless ────────────────────────────────────────
    chrome = _chrome_bin()
    if chrome:
        cmd = [
            chrome,
            "--headless=new",
            "--disable-gpu",
            "--no-sandbox",
            "--run-all-compositor-stages-before-draw",
            f"--print-to-pdf={pdf_path}",
            f"--print-to-pdf-no-header",
            str(html_path.resolve()),
        ]
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode == 0 and pdf_path.exists() and pdf_path.stat().st_size > 0:
            return True
        # --headless=new flag not supported on older Chrome; retry with legacy flag
        cmd[1] = "--headless"
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode == 0 and pdf_path.exists() and pdf_path.stat().st_size > 0:
            return True
        print(f"Chrome headless failed: {result.stderr[:200]}", file=sys.stderr)

    # ── Backend 2: weasyprint ─────────────────────────────────────────────
    try:
        import weasyprint
        weasyprint.HTML(filename=str(html_path)).write_pdf(str(pdf_path))
        return True
    except ImportError:
        result = subprocess.run(
            [sys.executable, "-m", "pip", "install", "--quiet", "weasyprint"],
            capture_output=True,
        )
        if result.returncode == 0:
            try:
                import importlib, weasyprint
                importlib.reload(weasyprint)
                weasyprint.HTML(filename=str(html_path)).write_pdf(str(pdf_path))
                return True
            except Exception:
                pass
    except Exception:
        pass

    return False


# ─────────────────────────────────────────────────────────────────────────────
# Tarball embedding
# ─────────────────────────────────────────────────────────────────────────────

def embed_into_tarball(pdf_path: Path, tarball_path: Path) -> None:
    """
    Repack tarball_path to include pdf_path as auditor-review.pdf.
    Updates MANIFEST.json inside the tarball with the PDF's SHA256.
    Overwrites tarball_path in-place via an adjacent .tmp file.
    """
    pdf_bytes  = pdf_path.read_bytes()
    pdf_sha256 = "sha256:" + hashlib.sha256(pdf_bytes).hexdigest()

    # Read all existing members into memory.
    members: list[tuple[tarfile.TarInfo, bytes | None]] = []
    prefix        = None
    manifest_idx  = None

    with tarfile.open(tarball_path) as tf:
        for i, member in enumerate(tf.getmembers()):
            if prefix is None and "/" in member.name:
                prefix = member.name.split("/")[0]
            fh   = tf.extractfile(member)
            data = fh.read() if fh else None
            members.append((member, data))
            if member.name.endswith("/MANIFEST.json") and data is not None:
                manifest_idx = i

    prefix = prefix or "agentcarousel-evidence"

    # Update MANIFEST.json to record the new file.
    if manifest_idx is not None:
        old_info, old_data = members[manifest_idx]
        manifest = json.loads(old_data)
        manifest.setdefault("files", []).append({
            "path": "auditor-review.pdf",
            "sha256": pdf_sha256,
        })
        new_data       = json.dumps(manifest, indent=2).encode()
        new_info       = copy.copy(old_info)
        new_info.size  = len(new_data)
        members[manifest_idx] = (new_info, new_data)

    # Write to a sibling .tmp file, then atomically replace.
    tmp = tarball_path.with_suffix(".tmp.tar.gz")
    try:
        with tarfile.open(tmp, "w:gz") as tf_out:
            for info, data in members:
                info = copy.copy(info)
                if data is not None:
                    info.size = len(data)
                    tf_out.addfile(info, io.BytesIO(data))
                else:
                    tf_out.addfile(info)
            pdf_info       = tarfile.TarInfo(name=f"{prefix}/auditor-review.pdf")
            pdf_info.size  = len(pdf_bytes)
            pdf_info.mtime = int(time.time())
            tf_out.addfile(pdf_info, io.BytesIO(pdf_bytes))
        tmp.replace(tarball_path)
    except Exception:
        tmp.unlink(missing_ok=True)
        raise


# ─────────────────────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────────────────────

def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate a human-readable auditor PDF for an agentcarousel bundle.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent("""\
            Examples:
              # Offline-only (no run data):
              ./venv/bin/python3 scripts/generate-auditor-pdf.py \\
                  --bundle fixtures/bundles/cmmc-assessor

              # With carousel run JSON files from carousel-evidence-5x.sh:
              ./venv/bin/python3 scripts/generate-auditor-pdf.py \\
                  --bundle fixtures/bundles/cmmc-assessor \\
                  --runs-dir reports/carousel-runs

              # With evidence tarballs from `agentcarousel export`:
              ./venv/bin/python3 scripts/generate-auditor-pdf.py \\
                  --bundle fixtures/bundles/cmmc-assessor \\
                  --tarball reports/evidence-packs/cmmc-assessor/agentcarousel-evidence-*.tar.gz
        """),
    )
    parser.add_argument(
        "--bundle", required=True, type=Path,
        help="Bundle directory (contains bundle.manifest.json)",
    )
    parser.add_argument(
        "--last", type=int, default=5, metavar="N",
        help="Auto-discover the N most recent bundle-matching runs from local history (default: 5). "
             "Ignored when --runs-dir or --tarball are supplied.",
    )
    parser.add_argument(
        "--runs-dir", type=Path, default=None,
        help="Directory of carousel run JSON files (from carousel-evidence-5x.sh). "
             "Overrides --last auto-discovery.",
    )
    parser.add_argument(
        "--tarball", type=Path, nargs="*", default=[],
        help="One or more evidence .tar.gz files (from agentcarousel export). "
             "Overrides --last auto-discovery.",
    )
    parser.add_argument(
        "--out", type=Path, default=None,
        help="Output path (default: auditor-package-<skill>-<version>.pdf / .html)",
    )
    parser.add_argument(
        "--embed", type=Path, default=None, metavar="TARBALL",
        help="After generating the PDF, repack this evidence .tar.gz to include "
             "auditor-review.pdf and update its MANIFEST.json. Typically the same "
             "path as --tarball. The tarball is overwritten in-place.",
    )
    args = parser.parse_args()

    bundle_dir = args.bundle if args.bundle.is_absolute() else REPO_ROOT / args.bundle
    if not bundle_dir.is_dir():
        print(f"error: bundle directory not found: {bundle_dir}", file=sys.stderr)
        return 1

    # ── Load source data ──────────────────────────────────────────────────
    manifest  = load_manifest(bundle_dir)
    fixture   = load_fixture(manifest)
    judge_prompt = load_judge_prompt(manifest)
    golden_files_raw = load_golden_files(fixture)

    manual_sources = args.runs_dir or (args.tarball or [])
    if manual_sources:
        runs = load_runs(args.runs_dir, args.tarball or [])
    else:
        runs = fetch_bundle_runs(bundle_dir, n=args.last)

    skill   = manifest.get("skill_or_agent", "bundle")
    version = manifest.get("bundle_version", "0.0.0")

    # ── Prepare template data ─────────────────────────────────────────────
    fixture_cases_by_id = {c.get("id", ""): c for c in fixture.get("cases", [])}
    cert_case_ids = set(manifest.get("certification_cases", []))
    variance      = rubric_variance(runs) if runs else {}
    transcripts   = build_transcripts(runs, fixture_cases_by_id) if runs else []

    # Section numbers shift depending on which optional sections are present
    # §1 instructions, §2 manifest, §3 scenarios, §4 judge?, §5 golden?, §6 runs?
    section_num = 3 + bool(judge_prompt) + bool(golden_files_raw) + bool(runs)

    manifest_json_hl = hl_json(json.dumps(manifest, indent=2))

    judge_prompt_html = md(judge_prompt) if judge_prompt else ""

    golden_files = []
    for gf in golden_files_raw:
        golden_files.append({
            "path": gf["path"],
            "content_html": md(gf["content"]),
        })

    trust_state_example = json.dumps({
        "trust_state": "Trusted",
        "auditor": "<Auditor Full Name>",
        "certified_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "attestation_url": f"https://registry.agentcarousel.com/attestations/{skill}-{version}.minisig",
        "expires_at": None,
    }, indent=2)
    trust_state_example_hl = hl_json(trust_state_example)

    generated_at = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")

    # ── Render template ───────────────────────────────────────────────────
    env = Environment(loader=BaseLoader(), autoescape=False)
    env.globals["weight_bar"]    = weight_bar
    env.globals["status_badge"]  = status_badge
    tmpl = env.from_string(HTML_TEMPLATE)

    html = tmpl.render(
        manifest=manifest,
        fixture=fixture,
        cert_case_ids=cert_case_ids,
        cert_cases=list(cert_case_ids),
        judge_prompt=judge_prompt,
        judge_prompt_html=judge_prompt_html,
        golden_files=golden_files,
        runs=runs,
        variance=variance,
        transcripts=transcripts,
        section_num=section_num,
        manifest_json_hl=manifest_json_hl,
        trust_state_example=trust_state_example_hl,
        pygments_css=PYGMENTS_CSS,
        generated_at=generated_at,
    )

    # ── Write HTML ────────────────────────────────────────────────────────
    stem = f"auditor-package-{skill}-{version}"
    html_path = args.out.with_suffix(".html") if args.out else REPO_ROOT / f"{stem}.html"
    html_path.write_text(html)
    print(f"HTML written: {html_path}")

    # ── Attempt PDF ───────────────────────────────────────────────────────
    pdf_path = args.out if args.out else REPO_ROOT / f"{stem}.pdf"
    if pdf_path.suffix != ".pdf":
        pdf_path = pdf_path.with_suffix(".pdf")

    pdf_ok = try_generate_pdf(html_path, pdf_path)
    if pdf_ok:
        print(f"PDF  written: {pdf_path}")
        html_path.unlink()  # clean up intermediate HTML when PDF succeeded
    else:
        print(
            f"\nPDF conversion unavailable. Open the HTML in a browser and use\n"
            f"  File → Print → Save as PDF\n"
            f"  {html_path}\n",
            file=sys.stderr,
        )

    if args.embed:
        embed_src = pdf_path if pdf_ok else html_path
        if not embed_src.exists():
            print(f"error: cannot embed — {embed_src} not found", file=sys.stderr)
            return 1
        embed_into_tarball(embed_src, args.embed)
        embedded_name = "auditor-review.pdf" if pdf_ok else "auditor-review.html"
        print(f"Embedded {embedded_name} → {args.embed}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
