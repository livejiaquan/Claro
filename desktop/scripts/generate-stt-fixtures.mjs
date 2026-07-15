#!/usr/bin/env node

// 只輸出音檔，不播放聲音。這批 TTS fixture 是 regression smoke，不能取代真人語音驗收。

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const corpusPath = resolve(here, "../tests/eval/stt_accuracy.json");
const args = process.argv.slice(2);
const outFlag = args.indexOf("--out");
const outDir = resolve(
  outFlag >= 0 && args[outFlag + 1]
    ? args[outFlag + 1]
    : join(tmpdir(), "claro-stt-eval"),
);
const force = args.includes("--force");
const fixtureManifestPath = join(outDir, ".claro-stt-fixtures.json");

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function sourceHash(testCase) {
  return sha256(JSON.stringify({
    generator_version: 2,
    id: testCase.id,
    reference: testCase.reference,
    voice: testCase.tts.voice,
    rate: testCase.tts.rate,
  }));
}

function run(command, commandArgs) {
  const result = spawnSync(command, commandArgs, { encoding: "utf8" });
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "unknown error").trim();
    throw new Error(`${command} failed: ${detail}`);
  }
}

const corpus = JSON.parse(readFileSync(corpusPath, "utf8"));
mkdirSync(outDir, { recursive: true });
let previousManifest = { version: 1, cases: {} };
if (existsSync(fixtureManifestPath)) {
  try {
    previousManifest = JSON.parse(readFileSync(fixtureManifestPath, "utf8"));
  } catch {
    // Manifest 壞掉時 fail closed：逐案重生，不能默默沿用未知來源音檔。
  }
}
const nextManifest = { version: 1, corpus_version: corpus.version, cases: {} };

let generated = 0;
let skipped = 0;
for (const testCase of corpus.cases) {
  const wavPath = join(outDir, `${testCase.id}.wav`);
  const aiffPath = join(outDir, `${testCase.id}.aiff`);
  const expectedSourceHash = sourceHash(testCase);
  const previous = previousManifest.cases?.[testCase.id];
  if (
    !force
    && previous?.source_sha256 === expectedSourceHash
    && existsSync(wavPath)
    && sha256(readFileSync(wavPath)) === previous.wav_sha256
  ) {
    nextManifest.cases[testCase.id] = previous;
    skipped += 1;
    continue;
  }

  rmSync(aiffPath, { force: true });
  run("say", [
    "-v",
    testCase.tts.voice,
    "-r",
    String(testCase.tts.rate),
    "-o",
    aiffPath,
    testCase.reference,
  ]);
  run("afconvert", ["-f", "WAVE", "-d", "LEI16@16000", aiffPath, wavPath]);
  rmSync(aiffPath, { force: true });
  nextManifest.cases[testCase.id] = {
    source_sha256: expectedSourceHash,
    wav_sha256: sha256(readFileSync(wavPath)),
  };
  generated += 1;
}

const manifestTemp = `${fixtureManifestPath}.tmp`;
writeFileSync(manifestTemp, `${JSON.stringify(nextManifest, null, 2)}\n`, { mode: 0o600 });
renameSync(manifestTemp, fixtureManifestPath);

console.log(`STT fixtures: ${generated} generated, ${skipped} reused`);
console.log(outDir);
