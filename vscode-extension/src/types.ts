export type CertTrack = 'none' | 'candidate' | 'stable' | 'trusted';
export type RiskTier = 'low' | 'medium' | 'high';
export type DataHandling = 'synthetic-only' | 'no-pii' | 'pii-reviewed';
export type EvaluatorKind = 'rules' | 'golden' | 'judge' | 'process';

export interface OutputCheck {
  kind: 'contains' | 'not_contains' | 'regex' | 'equals' | 'json_path';
  value: string;
}

export interface RubricItem {
  id: string;
  description: string;
  weight: number;
  auto_check?: OutputCheck;
}

export type EvaluatorConfig =
  | { evaluator: 'rules' }
  | { evaluator: 'golden'; golden_path: string; golden_threshold: number }
  | { evaluator: 'judge'; judge_prompt: string }
  | { evaluator: 'process'; process_cmd: string[] };

export interface InputMessage {
  role: string;
  content: string;
}

export interface CaseInput {
  messages: InputMessage[];
  context?: Record<string, unknown>;
  env_overrides?: Record<string, unknown>;
}

export interface CaseExpected {
  tool_sequence?: unknown[];
  output?: OutputCheck[];
  rubric?: RubricItem[];
}

export interface FixtureCase {
  id: string;
  description?: string;
  tags?: string[];
  timeout_secs?: number;
  seed?: number;
  input: CaseInput;
  expected: CaseExpected;
  evaluator_config?: EvaluatorConfig;
  /** Line number (0-based) in the source YAML file where this case's `- id:` appears. */
  lineNumber?: number;
}

export interface FixtureFile {
  filePath: string;
  schema_version: number;
  skill_or_agent: string;
  certification_track?: CertTrack;
  risk_tier?: RiskTier;
  data_handling?: DataHandling;
  bundle_id?: string;
  bundle_version?: string;
  defaults?: {
    timeout_secs?: number;
    tags?: string[];
    evaluator?: string;
    seed?: number;
  };
  cases: FixtureCase[];
}

export function resolveEvaluatorKind(c: FixtureCase, defaults?: FixtureFile['defaults']): EvaluatorKind {
  if (c.evaluator_config) {
    return c.evaluator_config.evaluator as EvaluatorKind;
  }
  const d = defaults?.evaluator;
  if (d === 'golden' || d === 'judge' || d === 'process') return d;
  return 'rules';
}
