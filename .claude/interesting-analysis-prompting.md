Much better clinically — Sonnet understands the content, catches the right things (the SI risk assessment in
   case 2 is actually excellent), and the overall medical reasoning is correct. But it's 0/7 for the same
  reason across every case: it's completely ignoring the prompt.md. No [T####] citations anywhere, no
  escalation alerts, no sign-off blocks, PHI in the audit event.

  The model is defaulting to its prior on "what a clinical note looks like" instead of following the specific
  format constraints. This is a prompt.md problem, not a model capability ceiling — Sonnet can absolutely do
  all of this, it just wasn't told forcefully enough.

  The specific gaps in the current prompt.md:
  - Citation requirement is described but never shown with an example, so the model ignores it
  - Escalation alert format is mentioned but there's no concrete template to follow
  - Sign-off block is listed in the output order but never shown literally
  - The audit event PHI rule is described but the model has no idea what a compliant event looks like

  The fix is to rewrite prompt.md to use imperative language, show format skeletons, and include worked
  mini-examples for the two hardest requirements. Want me to do that?

✻ Sautéed for 49s

❯ I do want you to do that. I also find your analysis interesting. Do you think that the judge model should
  provide this sort of rationale or summarization as to why they judged the way they did? In other words,
  maybe they should also evaluate the effectiveness of the prompt when they finish scoring all of the
  generator's prompts? What do you think

⏺ Yes, and I have a strong opinion on the design: it's a good idea but it needs to be a separate run-level
  artifact, not mixed into the per-case scoring.

  The per-case judge is doing one job: evaluate this output against this rubric. If you add prompt diagnosis
  to that, you risk two things — the judge starts scoring leniently when it decides the prompt is "to blame,"
  and the per-case output becomes a mix of evaluation and meta-analysis that's harder to parse
  programmatically.

  The right design is a second pass at the end of the run, after all cases are scored, that sees the full
  picture and answers: why is this run failing, and where does the fix live? It's uniquely positioned to
  distinguish three very different failure modes that all look like "FAIL" in the output:

  1. Prompt failure — generator ignores a format requirement that was underspecified. Fix: rewrite the prompt.
   (What's happening here.)
  2. Model ceiling — generator understood the requirement but couldn't execute it. Fix: upgrade the model.
  3. Fixture miscalibration — rubric threshold is too strict for the variance in valid outputs. Fix: adjust
  weights or thresholds.

  A human has to manually synthesize across 7 cases to figure out which of those three is happening. The judge
   already has everything it needs — the prompt, the outputs, the per-case scores — and could say: "6 of 7
  cases are missing [T####] citations entirely. This is a systematic failure. The prompt mentions the citation
   requirement but provides no example and uses descriptive rather than imperative language. Suggested fix:
  add a worked example and use ALL-CAPS or bold for the requirement." That's actionable in a way that per-case
   rationales aren't.

  The catch is cost — it's another judge call on the full run context. But it's probably the highest-value
  token spend in the pipeline if you're doing iterative prompt development, which is exactly this situation.