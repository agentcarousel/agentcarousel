import * as fs from 'fs';
import * as yaml from 'js-yaml';
import { FixtureFile, FixtureCase } from './types';

interface CaseLines {
  start: number;
  input?: number;
  output?: number;
  rubric?: number;
  evaluatorConfig?: number;
}

function scanCaseLines(raw: string): Map<string, CaseLines> {
  const map = new Map<string, CaseLines>();
  const lines = raw.split('\n');
  let currentId: string | null = null;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // Case start: "  - id: <value>" (2-space indent before dash, at top of cases list)
    const caseMatch = line.match(/^\s+-\s+id:\s+["']?(.+?)["']?\s*$/);
    if (caseMatch) {
      currentId = caseMatch[1].trim();
      map.set(currentId, { start: i });
      continue;
    }

    if (!currentId) continue;
    const entry = map.get(currentId)!;

    if (/^    input:\s*$/.test(line) && entry.input == null)            { entry.input = i; }
    else if (/^      output:\s*$/.test(line) && entry.output == null)   { entry.output = i; }
    else if (/^      rubric:\s*$/.test(line) && entry.rubric == null)   { entry.rubric = i; }
    else if (/^    evaluator_config:\s*$/.test(line) && entry.evaluatorConfig == null) { entry.evaluatorConfig = i; }
  }

  return map;
}

function mergeDefaults(c: FixtureCase, defaults: FixtureFile['defaults']): FixtureCase {
  if (!defaults) return c;
  return {
    ...c,
    timeout_secs: c.timeout_secs ?? defaults.timeout_secs,
    tags: c.tags ?? defaults.tags,
  };
}

export function parseFixtureFile(filePath: string): FixtureFile | null {
  let raw: string;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch {
    return null;
  }

  let doc: unknown;
  try {
    doc = yaml.load(raw);
  } catch {
    return null;
  }

  if (!doc || typeof doc !== 'object') return null;
  const d = doc as Record<string, unknown>;

  if (d['schema_version'] !== 1) return null;
  if (typeof d['skill_or_agent'] !== 'string') return null;
  if (!Array.isArray(d['cases'])) return null;

  const caseLineMap = scanCaseLines(raw);
  const defaults = (d['defaults'] as FixtureFile['defaults']) ?? undefined;

  const cases: FixtureCase[] = (d['cases'] as unknown[])
    .filter((c): c is Record<string, unknown> => typeof c === 'object' && c !== null)
    .map((c) => {
      const id = String(c['id'] ?? '');
      const lines = caseLineMap.get(id);
      const fc: FixtureCase = {
        id,
        description: c['description'] != null ? String(c['description']) : undefined,
        tags: Array.isArray(c['tags']) ? (c['tags'] as string[]) : undefined,
        timeout_secs: typeof c['timeout_secs'] === 'number' ? c['timeout_secs'] : undefined,
        seed: typeof c['seed'] === 'number' ? c['seed'] : undefined,
        input: (c['input'] as FixtureCase['input']) ?? { messages: [] },
        expected: (c['expected'] as FixtureCase['expected']) ?? {},
        evaluator_config: c['evaluator_config'] as FixtureCase['evaluator_config'],
        lineNumber:          lines?.start,
        inputLine:           lines?.input,
        outputLine:          lines?.output,
        rubricLine:          lines?.rubric,
        evaluatorConfigLine: lines?.evaluatorConfig,
      };
      return mergeDefaults(fc, defaults);
    });

  return {
    filePath,
    schema_version: 1,
    skill_or_agent: d['skill_or_agent'] as string,
    certification_track: d['certification_track'] as FixtureFile['certification_track'],
    risk_tier: d['risk_tier'] as FixtureFile['risk_tier'],
    data_handling: d['data_handling'] as FixtureFile['data_handling'],
    bundle_id: d['bundle_id'] as string | undefined,
    bundle_version: d['bundle_version'] as string | undefined,
    defaults,
    cases,
  };
}
