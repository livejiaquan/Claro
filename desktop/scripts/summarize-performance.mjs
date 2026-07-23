import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const DEFAULT_INPUT = join(homedir(), ".claro", "history.jsonl");

function percentile(values, fraction) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  const rank = Math.max(1, Math.ceil(sorted.length * fraction));
  return sorted[Math.min(rank - 1, sorted.length - 1)];
}

function summarizeMetric(values) {
  return {
    samples: values.length,
    p50_ms: percentile(values, 0.5),
    p95_ms: percentile(values, 0.95),
    max_ms: values.length === 0 ? null : Math.max(...values),
  };
}

function modeOf(record) {
  return record.polish?.mode ?? record.polish_mode ?? record.mode ?? "unknown";
}

export function summarizeHistory(content) {
  const groups = {
    all: [],
    raw: [],
    polished: [],
  };
  const stages = {
    stt: [],
    polish: [],
    focus_guard: [],
    inject: [],
  };
  let corruptLines = 0;
  let pastedEntries = 0;

  for (const line of content.split(/\r?\n/)) {
    if (!line.trim()) continue;
    let record;
    try {
      record = JSON.parse(line);
    } catch {
      corruptLines += 1;
      continue;
    }
    if (record.status !== "pasted") continue;
    pastedEntries += 1;

    const total = record.timings?.release_to_paste_ms;
    if (Number.isFinite(total)) {
      groups.all.push(total);
      const mode = modeOf(record);
      if (mode === "raw") groups.raw.push(total);
      else if (mode === "clean" || mode === "organize") groups.polished.push(total);
    }

    const stt = record.timings?.stt_ms ?? record.timings?.stt;
    const polish = record.timings?.polish_ms ?? record.timings?.polish;
    const focusGuard = record.timings?.focus_guard_ms;
    const inject = record.timings?.inject_ms;
    if (Number.isFinite(stt)) stages.stt.push(stt);
    if (Number.isFinite(polish)) stages.polish.push(polish);
    if (Number.isFinite(focusGuard)) stages.focus_guard.push(focusGuard);
    if (Number.isFinite(inject)) stages.inject.push(inject);
  }

  return {
    pasted_entries: pastedEntries,
    entries_with_release_to_paste: groups.all.length,
    corrupt_lines: corruptLines,
    release_to_paste: {
      all: summarizeMetric(groups.all),
      raw: summarizeMetric(groups.raw),
      polished: summarizeMetric(groups.polished),
    },
    stages: {
      stt: summarizeMetric(stages.stt),
      polish: summarizeMetric(stages.polish),
      focus_guard: summarizeMetric(stages.focus_guard),
      inject: summarizeMetric(stages.inject),
    },
  };
}

function parseArgs(argv) {
  let input = DEFAULT_INPUT;
  let json = false;
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--input") {
      input = argv[index + 1];
      index += 1;
    } else if (argv[index] === "--json") {
      json = true;
    } else {
      throw new Error(`未知參數：${argv[index]}`);
    }
  }
  return { input, json };
}

function formatMetric(label, metric) {
  if (metric.samples === 0) return `${label}: 無可用樣本`;
  return `${label}: n=${metric.samples} p50=${metric.p50_ms}ms p95=${metric.p95_ms}ms max=${metric.max_ms}ms`;
}

async function main() {
  const { input, json } = parseArgs(process.argv.slice(2));
  const content = await readFile(input, "utf8");
  const summary = summarizeHistory(content);
  if (json) {
    console.log(JSON.stringify(summary, null, 2));
    return;
  }
  console.log(`來源：${input}`);
  console.log(`成功貼上：${summary.pasted_entries}；含 release-to-paste：${summary.entries_with_release_to_paste}`);
  console.log(formatMetric("Raw 放開到貼上", summary.release_to_paste.raw));
  console.log(formatMetric("含整理放開到貼上", summary.release_to_paste.polished));
  console.log(formatMetric("STT", summary.stages.stt));
  console.log(formatMetric("文字整理", summary.stages.polish));
  console.log(formatMetric("焦點驗證", summary.stages.focus_guard));
  console.log(formatMetric("貼上交易", summary.stages.inject));
  if (summary.corrupt_lines > 0) {
    console.log(`略過損壞行：${summary.corrupt_lines}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
