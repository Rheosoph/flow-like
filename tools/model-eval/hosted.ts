import { resolve } from "node:path";
import { readFile, writeFile } from "node:fs/promises";
import { computeClassification } from "./normalize";
import type { AAModel, GlobalMaxes } from "./types";
const ROOT = "/Users/felix/Git/flow-like";
const r = JSON.parse(await readFile(resolve(ROOT, "tmp/results.json"), "utf-8"));
const models: AAModel[] = r.data, maxes: GlobalMaxes = r.globalMaxes;
const prices = models.map(m => m.pricing.price_1m_blended_3_to_1).filter((p): p is number => p != null && p > 0);
// Presets whose underlying model IS in the index: link the real row rather than guess.
const LINK: Record<string, string> = {
  "Claude Haiku": "claude-4-5-haiku",
  "Hunyuan": "hy3",
  "Mercury": "mercury-2",
  "Roleplay": "minimax-m2",
};
const out: Record<string, unknown> = {};
for (const [name, slug] of Object.entries(LINK)) {
  const m = models.find(x => x.slug === slug);
  if (!m) { console.error(`MISSING ${slug}`); continue; }
  const { classification } = computeClassification(m, maxes, prices, false);
  out[name] = { slug, classification, intelligence: m.evaluations.artificial_analysis_intelligence_index };
  const c = classification as any;
  console.error(`${name.padEnd(15)} -> ${slug.padEnd(20)} AAII ${String(m.evaluations.artificial_analysis_intelligence_index).padStart(5)}  cod=${c.coding} rea=${c.reasoning} fac=${c.factuality}`);
}
await writeFile("/private/tmp/claude-501/-Users-felix-Git-flow-like/2c430ae4-2f1e-40a3-b504-fa932f513ae2/scratchpad/hosted-linked.json", JSON.stringify(out, null, 1));
