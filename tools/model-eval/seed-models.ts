#!/usr/bin/env bun
/**
 * Seed LlmModel records from Artificial Analysis API data.
 * Only upserts LlmModel rows — does NOT touch Bit parameters.
 *
 * Usage: bun run tools/model-eval/seed-models.ts
 */
import { resolve } from "path";
import { config } from "dotenv";
import { disconnect, upsertLlmModel } from "./db";
import { fetchModelsWithCache } from "./fetch";

const ROOT_DIR = resolve(import.meta.dir, "../..");
config({ path: resolve(ROOT_DIR, ".env") });

const apiKey = process.env.ARTIFICIAL_ANALYSIS;
if (!apiKey) {
	console.error("Missing ARTIFICIAL_ANALYSIS in .env");
	process.exit(1);
}

async function main() {
	console.log("=== Seed LlmModel Records ===\n");

	const { models } = await fetchModelsWithCache(apiKey!, ROOT_DIR);

	console.log(`\n[db] Upserting ${models.length} LlmModel records...`);
	let count = 0;
	for (const m of models) {
		await upsertLlmModel(m);
		count++;
		if (count % 50 === 0) console.log(`  ...${count}/${models.length}`);
	}

	console.log(`[db] Done — upserted ${count} LlmModel records`);
	await disconnect();
}

main().catch((err) => {
	console.error("Fatal error:", err);
	disconnect().catch(() => {});
	process.exit(1);
});
