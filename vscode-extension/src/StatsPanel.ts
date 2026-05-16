import * as vscode from 'vscode';
import * as AgcRunner from './AgcRunner';
import type { StatsResult } from './AgcRunner';

export class StatsPanel {
  private static current: StatsPanel | undefined;
  private readonly panel: vscode.WebviewPanel;

  private constructor(private readonly context: vscode.ExtensionContext) {
    this.panel = vscode.window.createWebviewPanel(
      'agentcarousel.stats',
      'AgentCarousel — Stats',
      vscode.ViewColumn.One,
      { enableScripts: false, retainContextWhenHidden: true },
    );
    this.panel.onDidDispose(() => { StatsPanel.current = undefined; }, null, context.subscriptions);
  }

  static async show(context: vscode.ExtensionContext, skill?: string): Promise<void> {
    if (StatsPanel.current) {
      StatsPanel.current.panel.reveal();
    } else {
      StatsPanel.current = new StatsPanel(context);
    }
    await StatsPanel.current.load(skill);
  }

  private async load(skill?: string): Promise<void> {
    this.panel.webview.html = loading();
    try {
      const stats = await AgcRunner.runStats(skill);
      this.panel.webview.html = buildHtml(stats);
    } catch (err) {
      const msg = String(err);
      this.panel.webview.html = errorHtml(
        msg.includes('unrecognized subcommand')
          ? 'agc stats requires agc >= 0.5.3 — run `agc update` to upgrade.'
          : msg,
      );
    }
  }
}

function buildHtml(s: StatsResult): string {
  const sparkline = buildSparkline(s.pass_rate_trend.map((p) => p.pass_rate));
  const latencyLine = buildSparkline(s.mean_latency_trend_ms.map((v) => v / Math.max(...s.mean_latency_trend_ms, 1)));

  const flakiestRows = s.flakiest_cases.map((c) =>
    `<tr><td class="mono">${esc(c.case_id)}</td><td>${(c.flakiness * 100).toFixed(1)}%</td></tr>`,
  ).join('');

  const trendRows = s.pass_rate_trend.slice(0, 10).map((p) => {
    const pct = (p.pass_rate * 100).toFixed(1);
    const bar = Math.round(p.pass_rate * 40);
    return `<tr><td>${esc(p.at)}</td><td>${pct}%</td><td><span class="bar" style="width:${bar * 6}px"></span></td></tr>`;
  }).join('');

  return page(`
    <h2>Stats <span class="count">${s.run_count} runs</span></h2>

    <div class="charts-row">
      <div class="chart-block">
        <div class="chart-label">Pass Rate Trend</div>
        <svg width="280" height="60" class="sparkline">${sparkline}</svg>
      </div>
      <div class="chart-block">
        <div class="chart-label">Latency Trend (relative)</div>
        <svg width="280" height="60" class="sparkline">${latencyLine}</svg>
      </div>
    </div>

    ${trendRows ? `
    <h3>Recent Runs</h3>
    <table>
      <thead><tr><th>Time</th><th>Pass Rate</th><th></th></tr></thead>
      <tbody>${trendRows}</tbody>
    </table>` : ''}

    ${flakiestRows ? `
    <h3>Flakiest Cases</h3>
    <table>
      <thead><tr><th>Case</th><th>Flakiness</th></tr></thead>
      <tbody>${flakiestRows}</tbody>
    </table>` : '<p class="meta">No flaky cases detected.</p>'}
  `);
}

function buildSparkline(values: number[]): string {
  if (values.length === 0) return '';
  const W = 280, H = 60, pad = 4;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const pts = values.map((v, i) => {
    const x = pad + (i / Math.max(values.length - 1, 1)) * (W - pad * 2);
    const y = pad + (1 - (v - min) / range) * (H - pad * 2);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  return `<polyline points="${pts.join(' ')}" fill="none" stroke="var(--vscode-textLink-foreground)" stroke-width="2" stroke-linejoin="round"/>`;
}

function esc(s: string): string {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function loading(): string { return page('<p class="meta">Loading stats…</p>'); }
function errorHtml(msg: string): string { return page(`<p class="error">${esc(msg)}</p>`); }

function page(body: string): string {
  return `<!DOCTYPE html><html><head><meta charset="UTF-8">
<style>
  body { font-family: var(--vscode-font-family); font-size: var(--vscode-font-size); color: var(--vscode-foreground); background: var(--vscode-editor-background); padding: 16px; }
  h2, h3 { margin: 16px 0 8px; }
  table { border-collapse: collapse; width: 100%; margin-bottom: 16px; }
  th, td { text-align: left; padding: 5px 10px; border-bottom: 1px solid var(--vscode-widget-border, #333); }
  .mono { font-family: var(--vscode-editor-font-family, monospace); font-size: 0.9em; }
  .count { color: var(--vscode-descriptionForeground); font-size: 0.85em; font-weight: normal; }
  .meta { color: var(--vscode-descriptionForeground); }
  .error { color: var(--vscode-errorForeground); }
  .charts-row { display: flex; gap: 24px; flex-wrap: wrap; margin: 16px 0; }
  .chart-block { background: var(--vscode-input-background); border-radius: 6px; padding: 12px; }
  .chart-label { font-size: 0.8em; color: var(--vscode-descriptionForeground); margin-bottom: 6px; }
  .sparkline { display: block; }
  .bar { display: inline-block; height: 10px; background: var(--vscode-textLink-foreground); border-radius: 2px; vertical-align: middle; }
</style></head><body>${body}</body></html>`;
}
