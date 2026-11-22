import * as cp from 'child_process';
import * as vscode from 'vscode';
import { unwrapData, validateOutcome, type ValidateOutcome } from './agcOutput';

export type { ValidateMessage, ValidateOutcome } from './agcOutput';

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

/**
 * Binaries to try, in order. `agc` is the default, but the curl installer only offers it as a
 * shell alias, which spawn() cannot see, so the default falls back to `agentcarousel`.
 */
function agcBinaries(): string[] {
  const configured = vscode.workspace.getConfiguration('agentcarousel').get<string>('agcPath') || 'agc';
  return configured === 'agc' ? ['agc', 'agentcarousel'] : [configured];
}

function cwd(): string {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? process.cwd();
}

function run(args: string[], workingDir: string, okCodes: number[] = [0, 1]): Promise<string> {
  return spawnFirst(agcBinaries(), ['--no-color', ...args], workingDir, okCodes);
}

function spawnFirst(bins: string[], args: string[], workingDir: string, okCodes: number[]): Promise<string> {
  const [bin, ...fallbacks] = bins;
  return new Promise((resolve, reject) => {
    let settled = false;
    const proc = cp.spawn(bin, args, { cwd: workingDir });
    const chunks: Buffer[] = [];
    const errChunks: Buffer[] = [];
    proc.stdout.on('data', (d: Buffer) => chunks.push(d));
    proc.stderr.on('data', (d: Buffer) => errChunks.push(d));
    proc.on('close', (code) => {
      if (settled) return;
      settled = true;
      const stdout = Buffer.concat(chunks).toString('utf8');
      if (code !== null && okCodes.includes(code)) {
        resolve(stdout);
      } else {
        const stderr = Buffer.concat(errChunks).toString('utf8');
        reject(new Error(`${bin} exited ${code}: ${stderr || stdout}`));
      }
    });
    proc.on('error', (err: NodeJS.ErrnoException) => {
      if (settled) return;
      settled = true;
      if (err.code === 'ENOENT' && fallbacks.length > 0) {
        spawnFirst(fallbacks, args, workingDir, okCodes).then(resolve, reject);
      } else {
        reject(new Error(`Failed to spawn ${bin}: ${err.message}`));
      }
    });
  });
}

/** Runs `agc validate --json` on one file. Exit 2 means validation failed. */
export async function validate(filePath: string): Promise<ValidateOutcome> {
  const out = await run(['validate', '--json', filePath], cwd(), [0, 2]);
  return validateOutcome(out);
}

export async function reportList(limit = 50): Promise<RunListing[]> {
  const out = await run(['report', 'list', '--limit', String(limit), '--json'], cwd());
  return unwrapData<{ runs: RunListing[] }>(out).runs;
}

export async function reportShow(runId: string): Promise<RunDetail> {
  const out = await run(['report', 'show', '--json', runId], cwd());
  return unwrapData<RunDetail>(out);
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
