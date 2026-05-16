import * as vscode from 'vscode';
import * as path from 'path';
import * as AgcRunner from './AgcRunner';

export class DiagnosticsProvider implements vscode.Disposable {
  private readonly collection: vscode.DiagnosticCollection;
  private readonly subscriptions: vscode.Disposable[] = [];

  constructor() {
    this.collection = vscode.languages.createDiagnosticCollection('agentcarousel');

    const watcher = vscode.workspace.createFileSystemWatcher('**/fixtures/**/*.yaml');
    watcher.onDidChange((uri) => this.lintFile(uri.fsPath), null, this.subscriptions);
    watcher.onDidCreate((uri) => this.lintFile(uri.fsPath), null, this.subscriptions);
    watcher.onDidDelete((uri) => this.collection.delete(uri), null, this.subscriptions);
    this.subscriptions.push(watcher);
  }

  async lintAll(): Promise<void> {
    const uris = await vscode.workspace.findFiles('fixtures/**/*.yaml');
    await Promise.all(uris.map((u) => this.lintFile(u.fsPath)));
  }

  async lintFile(filePath: string): Promise<void> {
    const uri = vscode.Uri.file(filePath);
    const diags: vscode.Diagnostic[] = [];

    try {
      const validateResult = await AgcRunner.validate(filePath, 'json');
      for (const msg of validateResult.messages ?? []) {
        diags.push(this.toDiagnostic(msg, 'agc validate'));
      }
    } catch {
      // agc not on PATH or fixture dir doesn't exist yet — silently skip
    }

    try {
      const lintResult = await AgcRunner.lint([filePath]);
      for (const msg of lintResult.messages ?? []) {
        diags.push(this.toDiagnostic(msg, 'agc lint'));
      }
    } catch {
      // same — skip silently
    }

    this.collection.set(uri, diags);
  }

  private toDiagnostic(
    msg: { line: number; col: number; level: string; message: string },
    source: string,
  ): vscode.Diagnostic {
    const line = Math.max(0, (msg.line ?? 1) - 1);
    const col = Math.max(0, (msg.col ?? 1) - 1);
    const range = new vscode.Range(line, col, line, col + 1);
    const severity =
      msg.level === 'error'
        ? vscode.DiagnosticSeverity.Error
        : msg.level === 'warning'
        ? vscode.DiagnosticSeverity.Warning
        : vscode.DiagnosticSeverity.Information;
    const diag = new vscode.Diagnostic(range, msg.message, severity);
    diag.source = source;
    return diag;
  }

  dispose(): void {
    this.collection.dispose();
    for (const sub of this.subscriptions) sub.dispose();
  }
}
