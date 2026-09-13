import { resolve } from "node:path";
import { readFile } from "node:fs/promises";
import { computeClassification } from "./normalize";
import type { AAModel, GlobalMaxes } from "./types";
const ROOT = "/Users/felix/Git/flow-like";
const r = JSON.parse(await readFile(resolve(ROOT, "tmp/results.json"), "utf-8"));
const models: AAModel[] = r.data, maxes: GlobalMaxes = r.globalMaxes;
const prices = models.map(m => m.pricing.price_1m_blended_3_to_1).filter((p): p is number => p != null && p > 0);
const rows: {x: number[]; y: number}[] = [];
for (const m of models) {
  const y = m.evaluations.artificial_analysis_intelligence_index;
  if (y == null || y <= 0) continue;
  const { classification: c } = computeClassification(m, maxes, prices, false);
  if (!(c.coding > 0 && c.reasoning > 0 && c.factuality > 0)) continue;
  rows.push({ x: [1, c.coding, c.reasoning, c.factuality], y });
}
// ordinary least squares via normal equations, 4 unknowns
const n = 4;
const A = Array.from({length: n}, () => new Array(n).fill(0));
const b = new Array(n).fill(0);
for (const {x, y} of rows) {
  for (let i = 0; i < n; i++) { b[i] += x[i]*y; for (let j = 0; j < n; j++) A[i][j] += x[i]*x[j]; }
}
for (let i = 0; i < n; i++) {
  let p = i; for (let k = i+1; k < n; k++) if (Math.abs(A[k][i]) > Math.abs(A[p][i])) p = k;
  [A[i], A[p]] = [A[p], A[i]]; [b[i], b[p]] = [b[p], b[i]];
  for (let k = i+1; k < n; k++) { const f = A[k][i]/A[i][i]; for (let j = i; j < n; j++) A[k][j] -= f*A[i][j]; b[k] -= f*b[i]; }
}
const w = new Array(n).fill(0);
for (let i = n-1; i >= 0; i--) { let s = b[i]; for (let j = i+1; j < n; j++) s -= A[i][j]*w[j]; w[i] = s/A[i][i]; }
const pred = (c: number[]) => w[0] + w[1]*c[0] + w[2]*c[1] + w[3]*c[2];
const errs = rows.map(r => Math.abs(pred([r.x[1], r.x[2], r.x[3]]) - r.y));
errs.sort((a,b)=>a-b);
console.log(`fitted on ${rows.length} benchmarked models`);
console.log(`intelligence = ${w[0].toFixed(2)} + ${w[1].toFixed(2)}*coding + ${w[2].toFixed(2)}*reasoning + ${w[3].toFixed(2)}*factuality`);
console.log(`median abs error ${errs[Math.floor(errs.length/2)].toFixed(2)}, 90th pct ${errs[Math.floor(errs.length*0.9)].toFixed(2)}`);
console.log("\nsanity against known models:");
for (const s of ["claude-opus-5","qwen3-8-27b","gemma-4-31b","qwen3-5-9b","granite-4-2-8b","minicpm5-2b","llama-3-1-instruct-8b"]) {
  const m = models.find(x => x.slug === s); if (!m) continue;
  const { classification: c } = computeClassification(m, maxes, prices, false);
  console.log(`  ${s.padEnd(24)} actual ${String(m.evaluations.artificial_analysis_intelligence_index).padStart(5)}   fitted ${pred([c.coding,c.reasoning,c.factuality]).toFixed(1).padStart(5)}`);
}
console.log("\nWEIGHTS " + JSON.stringify(w));
