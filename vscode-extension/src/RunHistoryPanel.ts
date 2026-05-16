import * as vscode from 'vscode';
import * as AgcRunner from './AgcRunner';
import type { RunDetail, RunListing } from './AgcRunner';

export class RunHistoryPanel {
  private static current: RunHistoryPanel | undefined;
  private readonly panel: vscode.WebviewPanel;
  private listings: RunListing[] = [];

  private constructor(private readonly context: vscode.ExtensionContext) {
    this.panel = vscode.window.createWebviewPanel(
      'agentcarousel.runHistory',
      'AgentCarousel — Run History',
      vscode.ViewColumn.One,
      { enableScripts: true, retainContextWhenHidden: true },
    );
    this.panel.onDidDispose(() => { RunHistoryPanel.current = undefined; }, null, context.subscriptions);
    this.panel.webview.onDidReceiveMessage(async (msg: { command: string; runId?: string }) => {
      if (msg.command === 'showDetail' && msg.runId) {
        await this.showDetail(msg.runId);
      } else if (msg.command === 'goBack') {
        await this.load();
      }
    }, null, context.subscriptions);
  }

  static async show(context: vscode.ExtensionContext): Promise<void> {
    if (RunHistoryPanel.current) {
      RunHistoryPanel.current.panel.reveal();
    } else {
      RunHistoryPanel.current = new RunHistoryPanel(context);
    }
    await RunHistoryPanel.current.load();
  }

  private async load(): Promise<void> {
    this.panel.webview.html = loadingHtml('Loading run history…');
    try {
      this.listings = await AgcRunner.reportList(50);
      this.panel.webview.html = this.listHtml();
    } catch (err) {
      this.panel.webview.html = errorHtml(String(err));
    }
  }

  private async showDetail(runId: string): Promise<void> {
    this.panel.webview.html = loadingHtml('Loading run…');
    try {
      const detail = await AgcRunner.reportShow(runId);
      this.panel.webview.html = this.detailHtml(detail);
    } catch (err) {
      this.panel.webview.html = errorHtml(String(err));
    }
  }

  private listHtml(): string {
    const rows = this.listings.map((r) => {
      const ts = new Date(r.started_at).toLocaleString();
      return `<tr class="row" data-id="${esc(r.id)}" onclick="pick('${esc(r.id)}')">
        <td class="mono">${esc(r.id)}</td>
        <td>${ts}</td>
      </tr>`;
    }).join('');

    return page(`
      <h2>Run History <span class="count">(${this.listings.length})</span></h2>
      <table>
        <thead><tr><th>Run ID</th><th>Started</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>
      <script>
        const vscode = acquireVsCodeApi();
        function pick(id) { vscode.postMessage({ command: 'showDetail', runId: id }); }
      </script>
    `);
  }

  private detailHtml(r: RunDetail): string {
    const s = r.summary;
    const passRate = (s.pass_rate * 100).toFixed(1);
    const caseRows = r.cases.map((c) => {
      const badge = statusBadge(c.status);
      const latency = c.metrics ? `${c.metrics.total_latency_ms}ms` : '—';
      const score = c.eval_scores ? (c.eval_scores.effectiveness_score * 100).toFixed(0) + '%' : '—';
      return `<tr><td class="mono">${esc(c.case_id)}</td><td>${badge}</td><td>${latency}</td><td>${score}</td></tr>`;
    }).join('');

    const p50 = s.latency_p50_ms != null ? `${s.latency_p50_ms.toFixed(0)}ms` : '—';
    const p95 = s.latency_p95_ms != null ? `${s.latency_p95_ms.toFixed(0)}ms` : '—';
    const p99 = s.latency_p99_ms != null ? `${s.latency_p99_ms.toFixed(0)}ms` : '—';

    return page(`
      <div class="back-link" onclick="goBack()">← Run History</div>
      <h2 class="mono">${esc(r.id)}</h2>
      <p class="meta">${new Date(r.started_at).toLocaleString()} · agc ${esc(r.agentcarousel_version)} · ${esc(r.command)}</p>
      <div class="stat-grid">
        <div class="stat"><span class="num">${passRate}%</span><span class="lbl">pass rate</span></div>
        <div class="stat"><span class="num">${s.passed}/${s.total}</span><span class="lbl">passed</span></div>
        <div class="stat"><span class="num">${s.mean_latency_ms.toFixed(0)}ms</span><span class="lbl">mean latency</span></div>
        <div class="stat"><span class="num">${p50}</span><span class="lbl">p50</span></div>
        <div class="stat"><span class="num">${p95}</span><span class="lbl">p95</span></div>
        <div class="stat"><span class="num">${p99}</span><span class="lbl">p99</span></div>
        ${s.mean_effectiveness_score != null ? `<div class="stat"><span class="num">${(s.mean_effectiveness_score * 100).toFixed(0)}%</span><span class="lbl">effectiveness</span></div>` : ''}
      </div>
      <table>
        <thead><tr><th>Case</th><th>Status</th><th>Latency</th><th>Score</th></tr></thead>
        <tbody>${caseRows}</tbody>
      </table>
      <script>
        const vscode = acquireVsCodeApi();
        function goBack() { vscode.postMessage({ command: 'goBack' }); }
      </script>
    `);
  }
}

function statusBadge(status: string): string {
  const map: Record<string, string> = {
    passed: 'badge-pass', failed: 'badge-fail', flaky: 'badge-flaky',
    skipped: 'badge-skip', timed_out: 'badge-fail', error: 'badge-fail',
  };
  return `<span class="badge ${map[status] ?? ''}">${status}</span>`;
}

function esc(s: string): string {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function loadingHtml(msg: string): string {
  return page(`<p class="loading">${msg}</p>`);
}

function errorHtml(msg: string): string {
  return page(`<p class="error">${esc(msg)}</p>`);
}

function page(body: string): string {
  return `<!DOCTYPE html><html><head><meta charset="UTF-8">
<style>
  body { font-family: var(--vscode-font-family); font-size: var(--vscode-font-size); color: var(--vscode-foreground); background: var(--vscode-editor-background); padding: 16px; }
  table { border-collapse: collapse; width: 100%; }
  th, td { text-align: left; padding: 6px 10px; border-bottom: 1px solid var(--vscode-widget-border, #333); }
  tr.row:hover, tbody tr:hover { background: var(--vscode-list-hoverBackground); cursor: pointer; }
  .mono { font-family: var(--vscode-editor-font-family, monospace); font-size: 0.9em; }
  .count { color: var(--vscode-descriptionForeground); font-size: 0.85em; }
  .meta { color: var(--vscode-descriptionForeground); margin: 4px 0 16px; }
  .stat-grid { display: flex; flex-wrap: wrap; gap: 12px; margin: 16px 0; }
  .stat { display: flex; flex-direction: column; align-items: center; background: var(--vscode-input-background); border-radius: 6px; padding: 10px 16px; min-width: 80px; }
  .stat .num { font-size: 1.4em; font-weight: 600; }
  .stat .lbl { font-size: 0.75em; color: var(--vscode-descriptionForeground); margin-top: 2px; }
  .badge { font-size: 0.8em; padding: 2px 7px; border-radius: 4px; font-weight: 600; }
  .badge-pass { background: #1e4d2b; color: #6fcf97; }
  .badge-fail { background: #4d1e1e; color: #eb5757; }
  .badge-flaky { background: #4d3a1e; color: #f2994a; }
  .badge-skip { background: #2a2a2a; color: #888; }
  .back-link { color: var(--vscode-textLink-foreground); cursor: pointer; margin-bottom: 12px; display: inline-block; }
  .loading { color: var(--vscode-descriptionForeground); }
  .error { color: var(--vscode-errorForeground); }
</style></head><body>${body}</body></html>`;
}
