import * as vscode from 'vscode';
import * as path from 'path';
import { FixtureFile, FixtureCase, resolveEvaluatorKind, EvaluatorKind } from './types';
import { parseFixtureFile } from './FixtureParser';

// ── Tree item kinds ──────────────────────────────────────────────────────────

export type TreeNodeKind = 'file' | 'case' | 'attribute';

export class FixtureTreeItem extends vscode.TreeItem {
  constructor(
    public readonly nodeKind: TreeNodeKind,
    label: string,
    collapsibleState: vscode.TreeItemCollapsibleState,
    public readonly fixture?: FixtureFile,
    public readonly fixtureCase?: FixtureCase,
  ) {
    super(label, collapsibleState);
  }
}

// ── Evaluator metadata ───────────────────────────────────────────────────────

const EVALUATOR_ICONS: Record<EvaluatorKind, string> = {
  rules: '$(symbol-keyword)',
  golden: '$(star-full)',
  judge: '$(comment-discussion)',
  process: '$(terminal)',
};

const EVALUATOR_LABELS: Record<EvaluatorKind, string> = {
  rules: 'rules',
  golden: 'golden',
  judge: 'judge',
  process: 'process',
};

// ── Provider ─────────────────────────────────────────────────────────────────

export class FixtureTreeProvider implements vscode.TreeDataProvider<FixtureTreeItem> {
  private _onDidChange = new vscode.EventEmitter<FixtureTreeItem | undefined>();
  readonly onDidChangeTreeData = this._onDidChange.event;

  private fixtures: FixtureFile[] = [];

  constructor(private readonly context: vscode.ExtensionContext) {
    this.loadFixtures();
  }

  refresh(): void {
    this.loadFixtures();
    this._onDidChange.fire(undefined);
  }

  private async loadFixtures(): Promise<void> {
    const config = vscode.workspace.getConfiguration('agentcarousel');
    const glob = config.get<string>('fixtureGlob') ?? 'fixtures/skills/**/*.yaml';
    const uris = await vscode.workspace.findFiles(glob);
    this.fixtures = uris
      .map((u) => parseFixtureFile(u.fsPath))
      .filter((f): f is FixtureFile => f !== null)
      .sort((a, b) => path.basename(a.filePath).localeCompare(path.basename(b.filePath)));
  }

  getTreeItem(element: FixtureTreeItem): vscode.TreeItem {
    return element;
  }

  async getChildren(element?: FixtureTreeItem): Promise<FixtureTreeItem[]> {
    if (!element) {
      if (this.fixtures.length === 0) await this.loadFixtures();
      return this.fixtures.map((f) => this.buildFileItem(f));
    }
    if (element.nodeKind === 'file' && element.fixture) {
      return element.fixture.cases.map((c) => this.buildCaseItem(c, element.fixture!));
    }
    if (element.nodeKind === 'case' && element.fixtureCase && element.fixture) {
      return this.buildAttributeItems(element.fixtureCase, element.fixture);
    }
    return [];
  }

  // ── File node ──────────────────────────────────────────────────────────────

  private buildFileItem(f: FixtureFile): FixtureTreeItem {
    const basename = path.basename(f.filePath);
    const trackBadge = f.certification_track ? f.certification_track : 'none';
    const tierBadge = f.risk_tier ?? '—';
    const item = new FixtureTreeItem(
      'file',
      basename,
      vscode.TreeItemCollapsibleState.Collapsed,
      f,
    );
    item.description = `[${trackBadge} | ${tierBadge} | ${f.cases.length} case${f.cases.length !== 1 ? 's' : ''}]`;
    item.tooltip = new vscode.MarkdownString(
      `**${f.skill_or_agent}**\n\n` +
      `- Track: \`${trackBadge}\`\n` +
      `- Risk: \`${tierBadge}\`\n` +
      `- Data: \`${f.data_handling ?? '—'}\`\n` +
      `- Cases: ${f.cases.length}\n\n` +
      `*${f.filePath}*`,
    );
    item.iconPath = new vscode.ThemeIcon('list-flat');
    item.contextValue = 'file';
    item.command = {
      command: 'agentcarousel.openInEditor',
      title: 'Open in Editor',
      arguments: [f.filePath, 0],
    };
    return item;
  }

  // ── Case node ──────────────────────────────────────────────────────────────

  private buildCaseItem(c: FixtureCase, f: FixtureFile): FixtureTreeItem {
    const shortId = c.id.includes('/') ? c.id.slice(c.id.lastIndexOf('/') + 1) : c.id;
    const evaluatorKind = resolveEvaluatorKind(c, f.defaults);
    const tags = (c.tags ?? []).filter((t) => t !== 'nightly').join(', ');

    const item = new FixtureTreeItem(
      'case',
      shortId,
      vscode.TreeItemCollapsibleState.Collapsed,
      f,
      c,
    );
    item.description = `[${EVALUATOR_LABELS[evaluatorKind]}]${tags ? '  ' + tags : ''}`;
    item.tooltip = new vscode.MarkdownString(
      `**${c.id}**\n\n` +
      (c.description ? `${c.description.trim()}\n\n` : '') +
      `- Evaluator: \`${evaluatorKind}\`\n` +
      (c.tags?.length ? `- Tags: ${c.tags.map((t) => `\`${t}\``).join(' ')}\n` : '') +
      (c.seed != null ? `- Seed: \`${c.seed}\`\n` : '') +
      (c.timeout_secs != null ? `- Timeout: \`${c.timeout_secs}s\`\n` : ''),
    );
    item.tooltip.isTrusted = true;
    item.iconPath = new vscode.ThemeIcon(EVALUATOR_ICONS[evaluatorKind].replace(/^\$\(/, '').replace(/\)$/, ''));
    item.contextValue = 'case';
    item.command = {
      command: 'agentcarousel.showCaseDetail',
      title: 'Show Case Detail',
      arguments: [c, f],
    };
    return item;
  }

  // ── Attribute nodes (children of a case) ──────────────────────────────────

  private buildAttributeItems(c: FixtureCase, f: FixtureFile): FixtureTreeItem[] {
    const items: FixtureTreeItem[] = [];

    // Input
    const firstUserMsg = c.input.messages.find((m) => m.role === 'user');
    const preview = firstUserMsg
      ? firstUserMsg.content.trim().slice(0, 72).replace(/\n/g, ' ') + (firstUserMsg.content.length > 72 ? '…' : '')
      : '(no user message)';
    const inputItem = new FixtureTreeItem('attribute', 'Input', vscode.TreeItemCollapsibleState.None, f, c);
    inputItem.description = preview;
    inputItem.iconPath = new vscode.ThemeIcon('comment');
    inputItem.tooltip = firstUserMsg?.content ?? '';
    inputItem.command = { command: 'agentcarousel.openInEditor', title: 'Open', arguments: [f.filePath, c.lineNumber ?? 0] };
    items.push(inputItem);

    // Output checks
    const checks = c.expected.output ?? [];
    if (checks.length > 0) {
      const byKind = checks.reduce<Record<string, number>>((acc, ch) => {
        acc[ch.kind] = (acc[ch.kind] ?? 0) + 1;
        return acc;
      }, {});
      const summary = Object.entries(byKind).map(([k, n]) => `${n} ${k}`).join(', ');
      const checkItem = new FixtureTreeItem('attribute', `Output checks (${checks.length})`, vscode.TreeItemCollapsibleState.None, f, c);
      checkItem.description = summary;
      checkItem.iconPath = new vscode.ThemeIcon('pass-filled');
      checkItem.command = { command: 'agentcarousel.openInEditor', title: 'Open', arguments: [f.filePath, c.lineNumber ?? 0] };
      items.push(checkItem);
    }

    // Rubric
    const rubric = c.expected.rubric ?? [];
    if (rubric.length > 0) {
      const totalWeight = rubric.reduce((s, r) => s + (r.weight ?? 0), 0);
      const rubricItem = new FixtureTreeItem('attribute', `Rubric (${rubric.length} items)`, vscode.TreeItemCollapsibleState.None, f, c);
      rubricItem.description = `weights sum: ${totalWeight.toFixed(2)}`;
      rubricItem.iconPath = new vscode.ThemeIcon('checklist');
      rubricItem.command = { command: 'agentcarousel.openInEditor', title: 'Open', arguments: [f.filePath, c.lineNumber ?? 0] };
      items.push(rubricItem);
    }

    // Evaluator config
    const evaluatorKind = resolveEvaluatorKind(c, f.defaults);
    const evalDetail = buildEvaluatorDetail(c, f, evaluatorKind);
    const evalItem = new FixtureTreeItem('attribute', `Evaluator: ${evaluatorKind}`, vscode.TreeItemCollapsibleState.None, f, c);
    evalItem.description = evalDetail;
    evalItem.iconPath = new vscode.ThemeIcon('beaker');
    evalItem.command = { command: 'agentcarousel.openInEditor', title: 'Open', arguments: [f.filePath, c.lineNumber ?? 0] };
    items.push(evalItem);

    return items;
  }
}

function buildEvaluatorDetail(c: FixtureCase, f: FixtureFile, kind: EvaluatorKind): string {
  const cfg = c.evaluator_config;
  if (!cfg) return kind;
  if (cfg.evaluator === 'golden') {
    return `threshold ${cfg.golden_threshold} · ${path.basename(cfg.golden_path)}`;
  }
  if (cfg.evaluator === 'judge') {
    return cfg.judge_prompt.trim().split('\n')[0].slice(0, 60);
  }
  if (cfg.evaluator === 'process') {
    return cfg.process_cmd.join(' ');
  }
  return kind;
}
