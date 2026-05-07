import * as vscode from 'vscode';
import * as path from 'path';
import { FixtureCase, FixtureFile, OutputCheck, RubricItem, resolveEvaluatorKind } from './types';

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
      if (msg.command === 'openInEditor') {
        vscode.commands.executeCommand('agentcarousel.openInEditor', this.fixture.filePath, this.fixtureCase.lineNumber ?? 0);
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

    const panel = vscode.window.createWebviewPanel(
      CaseDetailPanel.viewType,
      fixtureCase.id.includes('/') ? fixtureCase.id.slice(fixtureCase.id.lastIndexOf('/') + 1) : fixtureCase.id,
      vscode.ViewColumn.Active,
      { enableScripts: true, retainContextWhenHidden: true },
    );
    const instance = new CaseDetailPanel(panel, fixtureCase, fixture, context);
    CaseDetailPanel.panels.set(fixtureCase.id, instance);
    panel.onDidDispose(() => CaseDetailPanel.panels.delete(fixtureCase.id));
  }

  private render(): void {
    this.panel.webview.html = this.buildHtml();
  }

  private dispose(): void {
    this.disposables.forEach((d) => d.dispose());
  }

  // ── HTML construction ──────────────────────────────────────────────────────

  private buildHtml(): string {
    const c = this.fixtureCase;
    const f = this.fixture;
    const evaluatorKind = resolveEvaluatorKind(c, f.defaults);
    const nonce = Math.random().toString(36).slice(2);

    const tags = (c.tags ?? []).map((t) => `<span class="tag">${esc(t)}</span>`).join(' ');
    const metaLine = [
      c.seed != null ? `seed <code>${c.seed}</code>` : '',
      c.timeout_secs != null ? `timeout <code>${c.timeout_secs}s</code>` : '',
    ].filter(Boolean).join(' &nbsp;·&nbsp; ');

    const messagesHtml = c.input.messages.map((m) =>
      `<div class="message ${esc(m.role)}">
        <div class="role-label">${esc(m.role)}</div>
        <pre class="content">${esc(m.content.trim())}</pre>
      </div>`,
    ).join('');

    const checksHtml = (c.expected.output ?? []).length > 0
      ? `<section>
          <h2>Output Checks <span class="count">${c.expected.output!.length}</span></h2>
          <table class="checks-table">
            <thead><tr><th>Kind</th><th>Value</th></tr></thead>
            <tbody>
              ${(c.expected.output ?? []).map((ch) =>
                `<tr>
                  <td><span class="kind-badge kind-${esc(ch.kind)}">${esc(ch.kind)}</span></td>
                  <td><code>${esc(ch.value)}</code></td>
                </tr>`,
              ).join('')}
            </tbody>
          </table>
        </section>`
      : '';

    const rubricHtml = (c.expected.rubric ?? []).length > 0
      ? `<section>
          <h2>Rubric <span class="count">${c.expected.rubric!.length} items</span></h2>
          ${(c.expected.rubric ?? []).map((r) => buildRubricCard(r)).join('')}
        </section>`
      : '';

    const evaluatorHtml = buildEvaluatorSection(c, evaluatorKind);

    return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'nonce-${nonce}'; script-src 'nonce-${nonce}';">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${esc(c.id)}</title>
  <style nonce="${nonce}">${CSS}</style>
</head>
<body>
  <header>
    <div class="case-id">${esc(c.id)}</div>
    ${tags ? `<div class="tags">${tags}</div>` : ''}
    ${c.description ? `<p class="description">${esc(c.description.trim())}</p>` : ''}
    ${metaLine ? `<p class="meta">${metaLine}</p>` : ''}
    <button class="open-btn" onclick="openInEditor()">Open in Editor</button>
  </header>

  <section>
    <h2>Input <span class="count">${c.input.messages.length} message${c.input.messages.length !== 1 ? 's' : ''}</span></h2>
    <div class="messages">${messagesHtml}</div>
  </section>

  ${checksHtml}
  ${rubricHtml}
  ${evaluatorHtml}

  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();
    function openInEditor() {
      vscode.postMessage({ command: 'openInEditor' });
    }
  </script>
</body>
</html>`;
  }
}

// ── HTML helpers ─────────────────────────────────────────────────────────────

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
    <div class="weight-bar"><div class="weight-fill" style="width:${pct}%"></div></div>
    <p class="rubric-desc">${esc(r.description.trim())}</p>
    ${r.auto_check ? `<div class="auto-check"><span class="kind-badge kind-${esc(r.auto_check.kind)}">${esc(r.auto_check.kind)}</span> <code>${esc(r.auto_check.value)}</code></div>` : ''}
  </div>`;
}

function buildEvaluatorSection(c: FixtureCase, kind: string): string {
  const cfg = c.evaluator_config;
  let detail = '';
  if (cfg?.evaluator === 'golden') {
    detail = `<p><strong>Golden file:</strong> <code>${esc(cfg.golden_path)}</code></p>
              <p><strong>Threshold:</strong> <code>${cfg.golden_threshold}</code></p>`;
  } else if (cfg?.evaluator === 'judge') {
    detail = `<p><strong>Judge prompt:</strong></p><pre class="judge-prompt">${esc(cfg.judge_prompt.trim())}</pre>`;
  } else if (cfg?.evaluator === 'process') {
    detail = `<p><strong>Command:</strong> <code>${esc(cfg.process_cmd.join(' '))}</code></p>`;
  }
  return `<section>
    <h2>Evaluator</h2>
    <p><span class="kind-badge kind-${esc(kind)}">${esc(kind)}</span></p>
    ${detail}
  </section>`;
}

// ── Styles ────────────────────────────────────────────────────────────────────

const CSS = `
  :root {
    --radius: 4px;
    --gap: 16px;
  }
  body {
    font-family: var(--vscode-font-family);
    font-size: var(--vscode-font-size);
    color: var(--vscode-editor-foreground);
    background: var(--vscode-editor-background);
    padding: 20px;
    max-width: 900px;
    margin: 0 auto;
  }
  header {
    border-bottom: 1px solid var(--vscode-panel-border);
    padding-bottom: var(--gap);
    margin-bottom: var(--gap);
  }
  .case-id {
    font-size: 1.3em;
    font-weight: 600;
    color: var(--vscode-foreground);
    margin-bottom: 6px;
  }
  .description {
    color: var(--vscode-descriptionForeground);
    margin: 6px 0;
  }
  .meta {
    font-size: 0.85em;
    color: var(--vscode-descriptionForeground);
    margin: 4px 0;
  }
  .tags { margin: 6px 0; }
  .tag {
    display: inline-block;
    background: var(--vscode-badge-background);
    color: var(--vscode-badge-foreground);
    border-radius: var(--radius);
    padding: 1px 6px;
    font-size: 0.8em;
    margin-right: 4px;
  }
  .open-btn {
    margin-top: 10px;
    background: var(--vscode-button-background);
    color: var(--vscode-button-foreground);
    border: none;
    padding: 5px 14px;
    border-radius: var(--radius);
    cursor: pointer;
    font-size: 0.9em;
  }
  .open-btn:hover { background: var(--vscode-button-hoverBackground); }
  section { margin-bottom: 28px; }
  h2 {
    font-size: 1em;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--vscode-descriptionForeground);
    border-bottom: 1px solid var(--vscode-panel-border);
    padding-bottom: 4px;
    margin-bottom: 12px;
  }
  .count {
    background: var(--vscode-badge-background);
    color: var(--vscode-badge-foreground);
    border-radius: 10px;
    padding: 1px 7px;
    font-size: 0.8em;
    margin-left: 6px;
  }
  /* Messages */
  .messages { display: flex; flex-direction: column; gap: 10px; }
  .message { border-radius: var(--radius); overflow: hidden; }
  .message.user { border-left: 3px solid var(--vscode-focusBorder); }
  .message.assistant { border-left: 3px solid var(--vscode-gitDecoration-addedResourceForeground, #4caf50); }
  .message.system { border-left: 3px solid var(--vscode-gitDecoration-modifiedResourceForeground, #ff9800); opacity: 0.8; }
  .role-label {
    font-size: 0.75em;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    padding: 3px 8px;
    background: var(--vscode-sideBar-background);
    color: var(--vscode-descriptionForeground);
  }
  pre.content {
    background: var(--vscode-textCodeBlock-background, var(--vscode-sideBar-background));
    padding: 10px 12px;
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 0.9em;
    font-family: var(--vscode-editor-font-family);
    max-height: 300px;
    overflow-y: auto;
  }
  /* Checks table */
  .checks-table { width: 100%; border-collapse: collapse; font-size: 0.9em; }
  .checks-table th {
    text-align: left;
    padding: 4px 8px;
    background: var(--vscode-sideBar-background);
    font-weight: 600;
    border-bottom: 1px solid var(--vscode-panel-border);
  }
  .checks-table td {
    padding: 5px 8px;
    border-bottom: 1px solid var(--vscode-panel-border);
    vertical-align: top;
  }
  .checks-table td:last-child code {
    word-break: break-all;
    font-size: 0.85em;
  }
  /* Kind badges */
  .kind-badge {
    display: inline-block;
    padding: 1px 6px;
    border-radius: var(--radius);
    font-size: 0.8em;
    font-weight: 600;
    white-space: nowrap;
  }
  .kind-contains    { background: #1a5c2a; color: #7ed99c; }
  .kind-not_contains { background: #5c1a1a; color: #f08080; }
  .kind-regex       { background: #1a3a5c; color: #80b8f0; }
  .kind-equals      { background: #3a1a5c; color: #c080f0; }
  .kind-json_path   { background: #3a3a1a; color: #d4d480; }
  .kind-rules       { background: #2a2a2a; color: #aaaaaa; }
  .kind-golden      { background: #3d3000; color: #f0c040; }
  .kind-judge       { background: #2d1a40; color: #c080f0; }
  .kind-process     { background: #001f3d; color: #60a0e0; }
  /* Rubric cards */
  .rubric-card {
    border: 1px solid var(--vscode-panel-border);
    border-radius: var(--radius);
    padding: 10px 14px;
    margin-bottom: 10px;
  }
  .rubric-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 6px; }
  .rubric-id { font-weight: 600; font-size: 0.95em; }
  .rubric-weight { font-size: 0.85em; color: var(--vscode-descriptionForeground); }
  .weight-bar {
    height: 4px;
    background: var(--vscode-progressBar-background, #333);
    border-radius: 2px;
    margin-bottom: 8px;
    overflow: hidden;
  }
  .weight-fill {
    height: 100%;
    background: var(--vscode-focusBorder, #007acc);
    border-radius: 2px;
  }
  .rubric-desc { font-size: 0.9em; color: var(--vscode-descriptionForeground); margin: 6px 0; }
  .auto-check { margin-top: 6px; font-size: 0.85em; }
  .auto-check code { word-break: break-all; }
  /* Judge prompt */
  pre.judge-prompt {
    background: var(--vscode-textCodeBlock-background, var(--vscode-sideBar-background));
    padding: 12px;
    border-radius: var(--radius);
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 0.85em;
    max-height: 400px;
    overflow-y: auto;
    font-family: var(--vscode-editor-font-family);
  }
  code {
    background: var(--vscode-textCodeBlock-background);
    padding: 1px 4px;
    border-radius: 2px;
    font-family: var(--vscode-editor-font-family);
  }
`;
