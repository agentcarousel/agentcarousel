import * as vscode from 'vscode';
import { FixtureTreeProvider } from './FixtureTreeProvider';
import { CaseDetailPanel } from './CaseDetailPanel';
import { DiagnosticsProvider } from './DiagnosticsProvider';
import { RunHistoryPanel } from './RunHistoryPanel';
import { StatsPanel } from './StatsPanel';
import * as AgcRunner from './AgcRunner';
import { FixtureCase, FixtureFile } from './types';

export function activate(context: vscode.ExtensionContext): void {
  const provider = new FixtureTreeProvider(context);
  const diagnostics = new DiagnosticsProvider();
  const doctorChannel = vscode.window.createOutputChannel('AgentCarousel Doctor');

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
    diagnostics,

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
      const current = vscode.workspace.getConfiguration('agentcarousel').get<string>('fixtureGlob') ?? 'fixtures/**/*.yaml';
      const value = await vscode.window.showInputBox({
        title: 'AgentCarousel: Fixture File Pattern',
        prompt: 'File pattern for YAML fixture files (relative to workspace root)',
        value: current,
        placeHolder: 'fixtures/**/*.yaml',
      });
      if (value !== undefined) {
        await vscode.workspace.getConfiguration('agentcarousel').update('fixtureGlob', value, vscode.ConfigurationTarget.Workspace);
        provider.refresh();
      }
    }),

    vscode.commands.registerCommand('agentcarousel.lintFixtures', async () => {
      await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Window, title: 'AgentCarousel: linting fixtures…' },
        () => diagnostics.lintAll(),
      );
    }),

    vscode.commands.registerCommand('agentcarousel.runDoctor', async () => {
      doctorChannel.clear();
      doctorChannel.show(true);
      doctorChannel.appendLine('Running agc doctor…\n');
      try {
        const output = await AgcRunner.runDoctor();
        const parsed = JSON.parse(output);
        doctorChannel.appendLine(JSON.stringify(parsed, null, 2));
      } catch (err) {
        const msg = String(err);
        if (msg.includes('unrecognized subcommand')) {
          doctorChannel.appendLine('agc doctor requires agc >= 0.5.1. Run `agc update` to upgrade.');
        } else {
          doctorChannel.appendLine(`Error: ${msg}`);
        }
      }
    }),

    vscode.commands.registerCommand('agentcarousel.showRunHistory', () => {
      RunHistoryPanel.show(context).catch((err: unknown) => {
        vscode.window.showErrorMessage(`AgentCarousel: ${String(err)}`);
      });
    }),

    vscode.commands.registerCommand('agentcarousel.showStats', () => {
      StatsPanel.show(context).catch((err: unknown) => {
        const msg = String(err);
        if (msg.includes('unrecognized subcommand')) {
          vscode.window.showErrorMessage('agc stats requires agc >= 0.5.3. Run `agc update` to upgrade.');
        } else {
          vscode.window.showErrorMessage(`AgentCarousel Stats: ${msg}`);
        }
      });
    }),
  );
}

export function deactivate(): void {}
