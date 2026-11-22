import * as vscode from 'vscode';
import * as AgcRunner from './AgcRunner';
import { diagnosticLevel, type ValidateMessage } from './agcOutput';

export class DiagnosticsProvider implements vscode.Disposable {
  private readonly collection: vscode.DiagnosticCollection;
  private readonly subscriptions: vscode.Disposable[] = [];

  constructor() {
    this.collection = vscode.languages.createDiagnosticCollection('agentcarousel');

    const watcher = vscode.workspace.createFileSystemWatcher('**/fixtures/**/*.yaml');
    watcher.onDidChange((uri) => this.validateFile(uri.fsPath), null, this.subscriptions);
    watcher.onDidCreate((uri) => this.validateFile(uri.fsPath), null, this.subscriptions);
    watcher.onDidDelete((uri) => this.collection.delete(uri), null, this.subscriptions);
    this.subscriptions.push(watcher);
  }

  async validateAll(): Promise<void> {
    const uris = await vscode.workspace.findFiles('fixtures/**/*.yaml');
    await Promise.all(uris.map((u) => this.validateFile(u.fsPath)));
  }

  async validateFile(filePath: string): Promise<void> {
    const uri = vscode.Uri.file(filePath);
    const diags: vscode.Diagnostic[] = [];

    try {
      const outcome = await AgcRunner.validate(filePath);
      for (const msg of outcome.messages) {
        diags.push(this.toDiagnostic(msg));
      }
      if (outcome.failure) {
        // `agc validate --json` gives no per-line detail on failure, so flag the file as a whole.
        // Warning, not Error: it can also mean agc could not load its schema.
        const diag = new vscode.Diagnostic(
          new vscode.Range(0, 0, 0, 1),
          `agc validate failed: ${outcome.failure} Run \`agc validate ${vscode.workspace.asRelativePath(filePath)}\` in a terminal for details.`,
          vscode.DiagnosticSeverity.Warning,
        );
        diag.source = 'agc validate';
        diags.push(diag);
      }
    } catch {
      // agc not installed, or output we can't parse — show nothing rather than a misleading error.
    }

    this.collection.set(uri, diags);
  }

  private toDiagnostic(msg: ValidateMessage): vscode.Diagnostic {
    const line = Math.max(0, (msg.line ?? 1) - 1);
    const col = Math.max(0, (msg.col ?? 1) - 1);
    const range = new vscode.Range(line, col, line, col + 1);
    const level = diagnosticLevel(msg.level);
    const severity =
      level === 'error'
        ? vscode.DiagnosticSeverity.Error
        : level === 'warning'
        ? vscode.DiagnosticSeverity.Warning
        : vscode.DiagnosticSeverity.Information;
    const diag = new vscode.Diagnostic(range, msg.message, severity);
    diag.source = 'agc validate';
    return diag;
  }

  dispose(): void {
    this.collection.dispose();
    for (const sub of this.subscriptions) sub.dispose();
  }
}
