import * as cp from 'child_process';
import * as vscode from 'vscode';

export interface ValidateMessage {
  file: string;
  line: number;
  col: number;
  level: 'error' | 'warning' | 'info';
  message: string;
}

export interface ValidateResult {
  messages: ValidateMessage[];
  atf_summary?: Record<string, unknown>;
}

export interface LintMessage {
  file: string;
  line: number;
  col: number;
  level: 'error' | 'warning' | 'info';
  message: string;
}

export interface LintResult {
  messages: LintMessage[];
}

export interface RunListing {
  id: string;
  started_at: string;
}

export interface CaseResult {
  case_id: string;
  status: 'passed' | 'failed' | 'skipped' | 'flaky' | 'timed_out' | 'error';
  error?: string;
  metrics?: {
    total_latency_ms: number;
    llm_calls: number;
    tool_calls: number;
  };
  eval_scores?: {
    effectiveness_score: number;
    passed: boolean;
  };
}

export interface RunSummary {
  total: number;
  passed: number;
  failed: number;
  skipped: number;
  flaky: number;
  errored: number;
  timed_out: number;
  pass_rate: number;
  mean_latency_ms: number;
  mean_effectiveness_score?: number;
  latency_p50_ms?: number;
  latency_p95_ms?: number;
  latency_p99_ms?: number;
  overall_status: string;
}

export interface RunDetail {
  id: string;
  started_at: string;
  finished_at: string;
  command: string;
  agentcarousel_version: string;
  cases: CaseResult[];
  summary: RunSummary;
}

export interface StatsResult {
  run_count: number;
  pass_rate_trend: { at: string; pass_rate: number }[];
  mean_latency_trend_ms: number[];
  flakiest_cases: { case_id: string; flakiness: number }[];
}

export interface DoctorResult {
  checks: { name: string; status: 'ok' | 'warn' | 'fail'; message?: string }[];
}

function findAgcBinary(): string {
  return vscode.workspace.getConfiguration('agentcarousel').get<string>('agcPath') || 'agc';
}

function cwd(): string {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? process.cwd();
}

function run(args: string[], workingDir: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const bin = findAgcBinary();
    const proc = cp.spawn(bin, ['--no-color', ...args], { cwd: workingDir });
    const chunks: Buffer[] = [];
    const errChunks: Buffer[] = [];
    proc.stdout.on('data', (d: Buffer) => chunks.push(d));
    proc.stderr.on('data', (d: Buffer) => errChunks.push(d));
    proc.on('close', (code) => {
      const stdout = Buffer.concat(chunks).toString('utf8');
      if (code === 0 || code === 1) {
        resolve(stdout);
      } else {
        const stderr = Buffer.concat(errChunks).toString('utf8');
        reject(new Error(`agc exited ${code}: ${stderr || stdout}`));
      }
    });
    proc.on('error', (err) => reject(new Error(`Failed to spawn agc: ${err.message}`)));
  });
}

export async function validate(filePath: string, format: 'json' | 'sarif' = 'json'): Promise<ValidateResult> {
  const out = await run(['validate', '--format', format, filePath], cwd());
  return JSON.parse(out) as ValidateResult;
}

export async function lint(paths: string[]): Promise<LintResult> {
  const out = await run(['lint', '--format', 'json', ...paths], cwd());
  return JSON.parse(out) as LintResult;
}

export async function reportList(limit = 50): Promise<RunListing[]> {
  const out = await run(['report', 'list', '--limit', String(limit), '--json'], cwd());
  return JSON.parse(out) as RunListing[];
}

export async function reportShow(runId: string): Promise<RunDetail> {
  const out = await run(['report', 'show', '--json', runId], cwd());
  return JSON.parse(out) as RunDetail;
}

export async function runStats(skill?: string): Promise<StatsResult> {
  const args = ['stats', '--format', 'json'];
  if (skill) args.push('--skill', skill);
  const out = await run(args, cwd());
  return JSON.parse(out) as StatsResult;
}

export async function runDoctor(): Promise<string> {
  return run(['doctor', '--json'], cwd());
}

export async function runTest(paths: string[], filter?: string): Promise<string> {
  const args = ['test', '--format', 'json', ...paths];
  if (filter) args.push('--filter', filter);
  return run(args, cwd());
}

export async function runEval(paths: string[], filter?: string): Promise<string> {
  const args = ['eval', '--format', 'json', ...paths];
  if (filter) args.push('--filter', filter);
  return run(args, cwd());
}
