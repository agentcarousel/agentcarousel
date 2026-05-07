import * as vscode from 'vscode';
import { FixtureTreeProvider } from './FixtureTreeProvider';
import { CaseDetailPanel } from './CaseDetailPanel';
import { FixtureCase, FixtureFile } from './types';

export function activate(context: vscode.ExtensionContext): void {
  const provider = new FixtureTreeProvider(context);

  const treeView = vscode.window.createTreeView('agentcarousel-fixtures', {
    treeDataProvider: provider,
    showCollapseAll: true,
  });

  const watcher = vscode.workspace.createFileSystemWatcher('**/fixtures/**/*.yaml');
  watcher.onDidChange(() => provider.refresh(), null, context.subscriptions);
  watcher.onDidCreate(() => provider.refresh(), null, context.subscriptions);
  watcher.onDidDelete(() => provider.refresh(), null, context.subscriptions);

  context.subscriptions.push(
    treeView,
    watcher,

    vscode.commands.registerCommand('agentcarousel.refreshFixtures', () => {
      provider.refresh();
    }),

    vscode.commands.registerCommand('agentcarousel.showCaseDetail', (c: FixtureCase, f: FixtureFile) => {
      CaseDetailPanel.show(context, c, f);
    }),

    vscode.commands.registerCommand('agentcarousel.openInEditor', async (filePath: string, line: number) => {
      try {
        const doc = await vscode.workspace.openTextDocument(filePath);
        const pos = new vscode.Position(Math.max(0, line), 0);
        await vscode.window.showTextDocument(doc, {
          selection: new vscode.Range(pos, pos),
          preserveFocus: false,
        });
      } catch (err) {
        vscode.window.showErrorMessage(`AgentCarousel: could not open ${filePath}: ${String(err)}`);
      }
    }),

    vscode.commands.registerCommand('agentcarousel.configureGlob', async () => {
      const current = vscode.workspace.getConfiguration('agentcarousel').get<string>('fixtureGlob') ?? 'fixtures/skills/**/*.yaml';
      const value = await vscode.window.showInputBox({
        title: 'AgentCarousel: Fixture Glob Pattern',
        prompt: 'Glob pattern for YAML fixture files (relative to workspace root)',
        value: current,
        placeHolder: 'fixtures/skills/**/*.yaml',
      });
      if (value !== undefined) {
        await vscode.workspace.getConfiguration('agentcarousel').update('fixtureGlob', value, vscode.ConfigurationTarget.Workspace);
        provider.refresh();
      }
    }),
  );
}

export function deactivate(): void {}
