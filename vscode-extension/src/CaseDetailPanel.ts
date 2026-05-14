import * as crypto from 'crypto';
import * as vscode from 'vscode';
import * as path from 'path';
import { FixtureCase, FixtureFile, RubricItem, resolveEvaluatorKind } from './types';

export class CaseDetailPanel {
  static readonly viewType = 'agentcarousel.caseDetail';
  private static panels = new Map<string, CaseDetailPanel>();

  private readonly panel: vscode.WebviewPanel;
  private disposables: vscode.Disposable[] = [];

  private constructor(
    panel: vscode.WebviewPanel,
    private fixtureCase: FixtureCase,
    private fixture: FixtureFile,
    private readonly context: vscode.ExtensionContext,
  ) {
    this.panel = panel;
    this.panel.onDidDispose(() => this.dispose(), null, this.disposables);
    this.panel.webview.onDidReceiveMessage((msg) => {
      if (msg.command === 'openGoldenFile') {
        const cfg = this.fixtureCase.evaluator_config;
        if (cfg?.evaluator === 'golden') {
          const wsRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? path.dirname(this.fixture.filePath);
          vscode.commands.executeCommand('agentcarousel.openInEditor', path.join(wsRoot, cfg.golden_path), 0);
        }
      }

    }, null, this.disposables);
    this.render();
  }

  static show(context: vscode.ExtensionContext, fixtureCase: FixtureCase, fixture: FixtureFile): void {
    const existing = CaseDetailPanel.panels.get(fixtureCase.id);
    if (existing) {
      existing.panel.reveal();
      existing.fixtureCase = fixtureCase;
      existing.fixture = fixture;
      existing.render();
      return;
    }

    const shortId = fixtureCase.id.includes('/')
      ? fixtureCase.id.slice(fixtureCase.id.lastIndexOf('/') + 1)
      : fixtureCase.id;

    const panel = vscode.window.createWebviewPanel(
      CaseDetailPanel.viewType,
      shortId,
      vscode.ViewColumn.Active,
      { enableScripts: true, retainContextWhenHidden: true },
    );
    const instance = new CaseDetailPanel(panel, fixtureCase, fixture, context);
    CaseDetailPanel.panels.set(fixtureCase.id, instance);
    panel.onDidDispose(() => CaseDetailPanel.panels.delete(fixtureCase.id));
  }

  private render(): void {
    this.panel.webview.html = buildHtml(this.fixtureCase, this.fixture);
  }

  private dispose(): void {
    this.disposables.forEach((d) => d.dispose());
  }
}

// ── HTML ──────────────────────────────────────────────────────────────────────

function buildHtml(c: FixtureCase, f: FixtureFile): string {
  const nonce = crypto.randomBytes(16).toString('hex');
  const evaluatorKind = resolveEvaluatorKind(c, f.defaults);

  const skillName = f.skill_or_agent;
  const shortId = c.id.includes('/') ? c.id.slice(c.id.lastIndexOf('/') + 1) : c.id;

  const metaBadges = [
    f.certification_track ? `<span class="badge track-${esc(f.certification_track)}">${esc(f.certification_track)}</span>` : '',
    f.risk_tier ? `<span class="badge risk-${esc(f.risk_tier)}">${esc(f.risk_tier)} risk</span>` : '',
    f.data_handling ? `<span class="badge data">${esc(f.data_handling)}</span>` : '',
  ].filter(Boolean).join('');

  const tags = (c.tags ?? [])
    .filter((t) => !['nightly', 'certification'].includes(t))
    .map((t) => `<span class="tag">${esc(t)}</span>`)
    .join('');

  const certTag = (c.tags ?? []).includes('certification')
    ? `<span class="tag tag-cert">certification</span>`
    : '';

  // Registry link: extract agent name from bundle_id (e.g. "agentcarousel/customer-support" → "customer-support")
  const agentName = f.bundle_id
    ? f.bundle_id.includes('/') ? f.bundle_id.slice(f.bundle_id.lastIndexOf('/') + 1) : f.bundle_id
    : f.skill_or_agent;
  const registryUrl = f.bundle_version
    ? `https://agentcarousel.com/agents/${encodeURIComponent(agentName)}/${encodeURIComponent(f.bundle_version)}`
    : null;

  const metaItems = [
    c.seed != null ? `<span class="meta-item">seed <code>${c.seed}</code></span>` : '',
    c.timeout_secs != null ? `<span class="meta-item">timeout <code>${c.timeout_secs}s</code></span>` : '',
    `<span class="meta-item">evaluator <span class="kind-badge kind-${esc(evaluatorKind)}">${esc(evaluatorKind)}</span></span>`,
    f.bundle_version ? `<span class="meta-item">bundle <code>${esc(f.bundle_id ?? agentName)}@${esc(f.bundle_version)}</code></span>` : '',
  ].filter(Boolean).join('');

  const messagesHtml = c.input.messages.map((m) =>
    `<div class="message msg-${esc(m.role)}">
      <div class="msg-role">${esc(m.role)}</div>
      <pre class="msg-content">${esc(m.content.trim())}</pre>
    </div>`,
  ).join('');

  const checks = c.expected.output ?? [];
  const checksHtml = checks.length > 0
    ? `<section>
        <h2>Output Checks <span class="count">${checks.length}</span></h2>
        <table class="checks-table">
          <thead><tr><th>Kind</th><th>Value</th></tr></thead>
          <tbody>
            ${checks.map((ch) =>
              `<tr>
                <td><span class="kind-badge kind-${esc(ch.kind)}">${esc(ch.kind)}</span></td>
                <td><code>${esc(ch.value)}</code></td>
              </tr>`,
            ).join('')}
          </tbody>
        </table>
      </section>`
    : '';

  const rubric = c.expected.rubric ?? [];
  const totalWeight = rubric.reduce((s, r) => s + (r.weight ?? 0), 0);
  const rubricHtml = rubric.length > 0
    ? `<section>
        <h2>Rubric <span class="count">${rubric.length} items · ${Math.round(totalWeight * 100)}% weight</span></h2>
        <div class="rubric-grid">
          ${rubric.map((r) => buildRubricCard(r)).join('')}
        </div>
      </section>`
    : '';

  const evaluatorHtml = buildEvaluatorSection(c, evaluatorKind);

  const toolSequence = c.expected.tool_sequence ?? [];
  const toolHtml = toolSequence.length > 0
    ? `<section>
        <h2>Expected Tool Sequence <span class="count">${toolSequence.length} call${toolSequence.length !== 1 ? 's' : ''}</span></h2>
        <pre class="tool-seq">${esc(JSON.stringify(toolSequence, null, 2))}</pre>
      </section>`
    : '';

  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'nonce-${nonce}'; script-src 'nonce-${nonce}';">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${esc(shortId)}</title>
  <style nonce="${nonce}">${CSS}</style>
</head>
<body>
  <nav class="breadcrumb">
    <span class="bc-skill">${esc(skillName)}</span>
    <span class="bc-sep">›</span>
    <span class="bc-case">${esc(shortId)}</span>
    ${metaBadges ? `<span class="bc-spacer"></span>${metaBadges}` : ''}
  </nav>

  <header>
    <div class="header-row">
      <h1 class="case-id">${esc(c.id)}</h1>
      ${registryUrl ? `<a class="registry-link" href="${registryUrl}">View on Registry ↗</a>` : ''}
    </div>
    ${tags || certTag ? `<div class="tags">${certTag}${tags}</div>` : ''}
    ${c.description ? `<p class="description">${esc(c.description.trim())}</p>` : ''}
    ${metaItems ? `<div class="meta-row">${metaItems}</div>` : ''}
  </header>

  <section>
    <h2>Input (Prompt) <span class="count">${c.input.messages.length} message${c.input.messages.length !== 1 ? 's' : ''}</span></h2>
    <div class="messages">${messagesHtml}</div>
  </section>

  ${checksHtml}
  ${rubricHtml}
  ${toolHtml}
  ${evaluatorHtml}

  <footer>
    <span class="footer-logo">◎</span>
    <span>AgentCarousel — Quality Assurance and Trust for Autonomous AI</span>
    <a class="footer-link" href="https://agentcarousel.com">agentcarousel.com</a>
  </footer>

  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();
    function openGoldenFile() { vscode.postMessage({ command: 'openGoldenFile' }); }
  </script>
</body>
</html>`;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function esc(s: string): string {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function buildRubricCard(r: RubricItem): string {
  const pct = Math.round(r.weight * 100);
  return `<div class="rubric-card">
    <div class="rubric-header">
      <span class="rubric-id">${esc(r.id)}</span>
      <span class="rubric-weight">${pct}%</span>
    </div>
    <div class="weight-track"><div class="weight-fill" style="width:${pct}%"></div></div>
    <p class="rubric-desc">${esc(r.description.trim())}</p>
    ${r.auto_check
      ? `<div class="auto-check"><span class="kind-badge kind-${esc(r.auto_check.kind)}">${esc(r.auto_check.kind)}</span><code>${esc(r.auto_check.value)}</code></div>`
      : ''}
  </div>`;
}

function buildEvaluatorSection(c: FixtureCase, kind: string): string {
  const cfg = c.evaluator_config;
  let detail = '';
  if (cfg?.evaluator === 'golden') {
    detail = `
      <div class="eval-row"><strong>Golden file</strong>
        <button class="golden-link" onclick="openGoldenFile()">${esc(cfg.golden_path)}</button>
      </div>
      <div class="eval-row"><strong>Threshold</strong>
        <span><code>${cfg.golden_threshold}</code>
        <span class="eval-hint"> — minimum similarity score to pass (0.0 = anything passes · 1.0 = exact match)</span></span>
      </div>`;
  } else if (cfg?.evaluator === 'judge') {
    detail = `<div class="eval-row"><strong>Judge prompt</strong></div>
      <pre class="judge-prompt">${esc(cfg.judge_prompt.trim())}</pre>`;
  } else if (cfg?.evaluator === 'process') {
    detail = `<div class="eval-row"><strong>Command</strong><code>${esc(cfg.process_cmd.join(' '))}</code></div>`;
  } else {
    detail = `<div class="eval-row eval-hint">Assertion-based evaluation — no LLM judge or golden file required.</div>`;
  }
  return `<section>
    <h2>Evaluator</h2>
    <div class="eval-card">
      <div class="eval-kind-row">
        <span class="kind-badge kind-${esc(kind)}">${esc(kind)}</span>
      </div>
      ${detail}
    </div>
  </section>`;
}

// ── CSS ───────────────────────────────────────────────────────────────────────

const CSS = `
  :root {
    --ac-accent:  #5b7fff;
    --ac-radius:  6px;
    --ac-gap:     20px;
  }

  * { box-sizing: border-box; }

  body {
    font-family: var(--vscode-font-family);
    font-size: var(--vscode-font-size);
    color: var(--vscode-editor-foreground);
    background: var(--vscode-editor-background);
    padding: 24px 28px;
    max-width: 860px;
    margin: 0 auto;
    line-height: 1.55;
  }

  /* ── Breadcrumb ──────────────────────────────────────────────────── */
  .breadcrumb {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 0.8em;
    color: var(--vscode-descriptionForeground);
    margin-bottom: 14px;
    flex-wrap: wrap;
  }
  .bc-skill { font-weight: 600; opacity: 0.8; }
  .bc-sep   { opacity: 0.4; }
  .bc-case  { opacity: 0.9; }
  .bc-spacer { flex: 1; }

  /* ── Certification / risk badges ─────────────────────────────────── */
  .badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 8px;
    border-radius: 20px;
    font-size: 0.75em;
    font-weight: 600;
    letter-spacing: 0.02em;
    border: 1px solid transparent;
  }
  .track-trusted   { background: rgba(34,197,94,0.15);  color: #4ade80; border-color: rgba(34,197,94,0.3); }
  .track-stable    { background: rgba(59,130,246,0.15); color: #60a5fa; border-color: rgba(59,130,246,0.3); }
  .track-candidate { background: rgba(245,158,11,0.15); color: #fbbf24; border-color: rgba(245,158,11,0.3); }
  .track-none      { background: rgba(156,163,175,0.1); color: var(--vscode-descriptionForeground); }
  .risk-high   { background: rgba(239,68,68,0.12);  color: #f87171; border-color: rgba(239,68,68,0.25); }
  .risk-medium { background: rgba(245,158,11,0.12); color: #fbbf24; border-color: rgba(245,158,11,0.25); }
  .risk-low    { background: rgba(34,197,94,0.12);  color: #4ade80; border-color: rgba(34,197,94,0.25); }
  .data        { background: rgba(139,92,246,0.12); color: #a78bfa; border-color: rgba(139,92,246,0.25); }

  /* ── Header ──────────────────────────────────────────────────────── */
  header {
    padding-bottom: var(--ac-gap);
    margin-bottom: var(--ac-gap);
    border-bottom: 1px solid var(--vscode-panel-border);
  }
  .header-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  h1.case-id {
    font-size: 1.35em;
    font-weight: 700;
    margin: 0;
    line-height: 1.3;
    word-break: break-all;
  }
  .registry-link {
    flex-shrink: 0;
    color: var(--vscode-textLink-foreground);
    font-size: 0.85em;
    white-space: nowrap;
    text-decoration: none;
    opacity: 0.85;
  }
  .registry-link:hover { opacity: 1; text-decoration: underline; }

  .tags { margin: 6px 0; display: flex; flex-wrap: wrap; gap: 4px; }
  .tag {
    display: inline-block;
    background: var(--vscode-badge-background);
    color: var(--vscode-badge-foreground);
    border-radius: 4px;
    padding: 1px 7px;
    font-size: 0.77em;
    font-weight: 500;
  }
  .tag-cert {
    background: rgba(245,158,11,0.18);
    color: #fbbf24;
    border: 1px solid rgba(245,158,11,0.3);
  }

  .description {
    font-size: 0.93em;
    color: var(--vscode-descriptionForeground);
    margin: 8px 0 4px;
  }
  .meta-row {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    font-size: 0.82em;
    color: var(--vscode-descriptionForeground);
    margin-top: 6px;
  }
  .meta-item { display: flex; align-items: center; gap: 4px; }

  /* ── Sections ────────────────────────────────────────────────────── */
  section { margin-bottom: 28px; }
  h2 {
    font-size: 0.8em;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--vscode-descriptionForeground);
    border-bottom: 1px solid var(--vscode-panel-border);
    padding-bottom: 5px;
    margin-bottom: 14px;
  }
  .count {
    background: var(--vscode-badge-background);
    color: var(--vscode-badge-foreground);
    border-radius: 20px;
    padding: 0 7px;
    font-size: 0.85em;
    font-weight: 500;
    text-transform: none;
    letter-spacing: 0;
  }

  /* ── Messages ────────────────────────────────────────────────────── */
  .messages { display: flex; flex-direction: column; gap: 8px; }
  .message { border-radius: var(--ac-radius); overflow: hidden; border: 1px solid var(--vscode-panel-border); }
  .msg-user      { border-left: 3px solid var(--ac-accent); }
  .msg-assistant { border-left: 3px solid #4ade80; }
  .msg-system    { border-left: 3px solid #fbbf24; opacity: 0.85; }
  .msg-role {
    font-size: 0.7em;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    padding: 4px 10px;
    background: var(--vscode-sideBar-background);
    color: var(--vscode-descriptionForeground);
  }
  pre.msg-content {
    background: var(--vscode-textCodeBlock-background, var(--vscode-sideBar-background));
    padding: 10px 12px;
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 0.88em;
    font-family: var(--vscode-editor-font-family);
    max-height: 320px;
    overflow-y: auto;
  }

  /* ── Checks table ────────────────────────────────────────────────── */
  .checks-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.88em;
    border: 1px solid var(--vscode-panel-border);
    border-radius: var(--ac-radius);
    overflow: hidden;
  }
  .checks-table th {
    text-align: left;
    padding: 6px 10px;
    background: var(--vscode-sideBar-background);
    font-weight: 600;
    font-size: 0.85em;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--vscode-descriptionForeground);
    border-bottom: 1px solid var(--vscode-panel-border);
  }
  .checks-table td {
    padding: 6px 10px;
    border-bottom: 1px solid var(--vscode-panel-border);
    vertical-align: top;
  }
  .checks-table tr:last-child td { border-bottom: none; }
  .checks-table td:last-child code { word-break: break-all; font-size: 0.85em; }

  /* ── Kind badges ─────────────────────────────────────────────────── */
  .kind-badge {
    display: inline-block;
    padding: 1px 7px;
    border-radius: 4px;
    font-size: 0.78em;
    font-weight: 700;
    white-space: nowrap;
    letter-spacing: 0.02em;
  }
  .kind-contains     { background: rgba(34,197,94,0.15);   color: #4ade80; }
  .kind-not_contains { background: rgba(239,68,68,0.15);   color: #f87171; }
  .kind-regex        { background: rgba(59,130,246,0.15);  color: #60a5fa; }
  .kind-equals       { background: rgba(139,92,246,0.15);  color: #a78bfa; }
  .kind-json_path    { background: rgba(234,179,8,0.15);   color: #facc15; }
  .kind-rules        { background: rgba(156,163,175,0.12); color: #9ca3af; }
  .kind-golden       { background: rgba(245,158,11,0.18);  color: #fbbf24; }
  .kind-judge        { background: rgba(139,92,246,0.18);  color: #a78bfa; }
  .kind-process      { background: rgba(59,130,246,0.18);  color: #60a5fa; }

  /* ── Rubric ──────────────────────────────────────────────────────── */
  .rubric-grid { display: flex; flex-direction: column; gap: 10px; }
  .rubric-card {
    border: 1px solid var(--vscode-panel-border);
    border-radius: var(--ac-radius);
    padding: 12px 14px;
    position: relative;
  }
  .rubric-card::before {
    content: '';
    position: absolute;
    left: 0; top: 0; bottom: 0;
    width: 3px;
    background: var(--ac-accent);
    border-radius: var(--ac-radius) 0 0 var(--ac-radius);
  }
  .rubric-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 5px;
  }
  .rubric-id { font-weight: 700; font-size: 0.92em; }
  .rubric-weight {
    font-size: 0.82em;
    font-weight: 600;
    color: var(--ac-accent);
    opacity: 0.9;
  }
  .weight-track {
    height: 3px;
    background: var(--vscode-panel-border);
    border-radius: 2px;
    margin-bottom: 9px;
    overflow: hidden;
  }
  .weight-fill {
    height: 100%;
    background: var(--ac-accent);
    border-radius: 2px;
    opacity: 0.7;
  }
  .rubric-desc {
    font-size: 0.88em;
    color: var(--vscode-descriptionForeground);
    margin: 0 0 6px;
    line-height: 1.5;
  }
  .auto-check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 0.82em;
    margin-top: 4px;
  }
  .auto-check code { word-break: break-all; }

  /* ── Evaluator ───────────────────────────────────────────────────── */
  .eval-card {
    border: 1px solid var(--vscode-panel-border);
    border-radius: var(--ac-radius);
    padding: 14px 16px;
  }
  .eval-kind-row { margin-bottom: 12px; }
  .eval-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
    font-size: 0.9em;
    margin-bottom: 8px;
    flex-wrap: wrap;
  }
  .eval-row strong { min-width: 90px; color: var(--vscode-foreground); flex-shrink: 0; }
  .eval-hint { color: var(--vscode-descriptionForeground); font-size: 0.85em; font-style: italic; }
  pre.judge-prompt {
    background: var(--vscode-textCodeBlock-background, var(--vscode-sideBar-background));
    padding: 12px;
    border-radius: var(--ac-radius);
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 0.84em;
    max-height: 380px;
    overflow-y: auto;
    font-family: var(--vscode-editor-font-family);
    border: 1px solid var(--vscode-panel-border);
    margin-top: 4px;
  }
  pre.tool-seq {
    background: var(--vscode-textCodeBlock-background, var(--vscode-sideBar-background));
    padding: 12px;
    border-radius: var(--ac-radius);
    font-size: 0.85em;
    font-family: var(--vscode-editor-font-family);
    border: 1px solid var(--vscode-panel-border);
    overflow-x: auto;
    margin: 0;
  }

  /* ── Golden link ─────────────────────────────────────────────────── */
  .golden-link {
    background: none;
    border: none;
    color: var(--vscode-textLink-foreground);
    font-family: var(--vscode-editor-font-family);
    font-size: 0.9em;
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .golden-link:hover { color: var(--vscode-textLink-activeForeground); }

  /* ── Inline code ─────────────────────────────────────────────────── */
  code {
    background: var(--vscode-textCodeBlock-background);
    padding: 1px 5px;
    border-radius: 3px;
    font-family: var(--vscode-editor-font-family);
    font-size: 0.9em;
  }

  /* ── Footer ──────────────────────────────────────────────────────── */
  footer {
    margin-top: 40px;
    padding-top: 14px;
    border-top: 1px solid var(--vscode-panel-border);
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 0.78em;
    color: var(--vscode-descriptionForeground);
    opacity: 0.7;
  }
  .footer-logo { font-size: 1.2em; opacity: 0.6; }
  .footer-link {
    margin-left: auto;
    color: var(--vscode-textLink-foreground);
    text-decoration: none;
    opacity: 0.9;
  }
  .footer-link:hover { text-decoration: underline; }
`;
