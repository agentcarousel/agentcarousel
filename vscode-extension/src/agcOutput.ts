// Parsing for `agc --json` output. No `vscode` import, so it can be tested with plain Node.

/** The envelope every `agc --json` command prints. */
export interface AgcEnvelope<T> {
  ok: boolean;
  command?: string;
  data?: T;
  error?: { code: string; message: string; suggestions?: string[] };
}

/** One `agc validate` message. `level` is `WARN` or `ERROR` (case varies across versions). */
export interface ValidateMessage {
  file: string;
  line: number;
  col: number;
  level: string;
  message: string;
}

export interface ValidateOutcome {
  messages: ValidateMessage[];
  /** Set when `agc validate` reported a failure. The JSON output carries no per-line detail then. */
  failure?: string;
}

export type DiagnosticLevel = 'error' | 'warning' | 'info';

export function parseEnvelope<T>(stdout: string): AgcEnvelope<T> {
  const parsed: unknown = JSON.parse(stdout);
  if (typeof parsed !== 'object' || parsed === null || typeof (parsed as AgcEnvelope<T>).ok !== 'boolean') {
    throw new Error('unexpected agc output: no JSON envelope');
  }
  return parsed as AgcEnvelope<T>;
}

/** Returns `data`, or throws with the CLI's error message. */
export function unwrapData<T>(stdout: string): T {
  const envelope = parseEnvelope<T>(stdout);
  if (!envelope.ok || envelope.data === undefined) {
    throw new Error(envelope.error?.message ?? 'agc reported an error');
  }
  return envelope.data;
}

export function validateOutcome(stdout: string): ValidateOutcome {
  const envelope = parseEnvelope<{ messages?: ValidateMessage[] }>(stdout);
  if (envelope.ok) {
    return { messages: envelope.data?.messages ?? [] };
  }
  return { messages: [], failure: envelope.error?.message ?? 'agc validate failed' };
}

export function diagnosticLevel(level: string | undefined): DiagnosticLevel {
  const normalized = (level ?? '').toLowerCase();
  if (normalized.startsWith('err')) return 'error';
  if (normalized.startsWith('warn')) return 'warning';
  return 'info';
}
