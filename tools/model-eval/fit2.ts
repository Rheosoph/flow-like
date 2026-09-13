import { resolve } from "node:path";
import { readFile, writeFile } from "node:fs/promises";
import { computeClassification } from "./normalize";
import type { AAModel, GlobalMaxes } from "./types";
const ROOT = "/Users/felix/Git/flow-like";
const r = JSON.parse(await readFile(resolve(ROOT, "tmp/results.json"), "utf-8"));
const models: AAModel[] = r.data, maxes: GlobalMaxes = r.globalMaxes;
const prices = models.map(m => m.pricing.price_1m_blended_3_to_1).filter((p): p is number => p != null && p > 0);

// The three capability traits move together, so a single composite carries the signal
// and avoids the collinearity that made a linear fit give reasoning a negative weight.
const composite = (c: any) => (c.coding + c.reasoning + c.factuality) / 3;
const pts: {c: number; y: number}[] = [];
for (const m of models) {
  const y = m.evaluations.artificial_analysis_intelligence_index;
  if (y == null || y <= 0) continue;
  const { classification: c } = computeClassification(m, maxes, prices, false);
  if (!(c.coding > 0 && c.reasoning > 0 && c.factuality > 0)) continue;
  pts.push({ c: composite(c), y });
}
pts.sort((a, b) => a.c - b.c);

/** Median intelligence of the K benchmarked models with the closest composite. */
function lookup(x: number, K = 25): number {
  const near = [...pts].sort((a, b) => Math.abs(a.c - x) - Math.abs(b.c - x)).slice(0, K);
  const ys = near.map(p => p.y).sort((a, b) => a - b);
  return Math.round(ys[Math.floor(ys.length / 2)] * 10) / 10;
}

const errs = pts.map(p => Math.abs(lookup(p.c) - p.y)).sort((a, b) => a - b);
console.log(`fitted on ${pts.length} benchmarked models`);
console.log(`median abs error ${errs[Math.floor(errs.length/2)].toFixed(2)}, 90th pct ${errs[Math.floor(errs.length*0.9)].toFixed(2)}`);
console.log("\nsanity:");
for (const s of ["claude-opus-5","qwen3-8-27b","gemma-4-31b","qwen3-5-9b","granite-4-2-8b","minicpm5-2b","ling-3-0-tiny","llama-3-1-instruct-8b"]) {
  const m = models.find(x => x.slug === s); if (!m) continue;
  const { classification: c } = computeClassification(m, maxes, prices, false);
  console.log(`  ${s.padEnd(24)} actual ${String(m.evaluations.artificial_analysis_intelligence_index).padStart(5)}   mapped ${String(lookup(composite(c))).padStart(5)}   composite ${composite(c).toFixed(2)}`);
}
// emit the curve so the packager can apply it without this pipeline
const curve = [];
for (let x = 0.10; x <= 0.95001; x += 0.01) curve.push({ composite: Math.round(x*100)/100, intelligence: lookup(x) });
await writeFile("/private/tmp/claude-501/-Users-felix-Git-flow-like/2c430ae4-2f1e-40a3-b504-fa932f513ae2/scratchpad/intelligence-curve.json", JSON.stringify(curve));
console.log("\ncurve written");
