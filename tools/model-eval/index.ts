#!/usr/bin/env bun
import { resolve } from "path";
import { readFile, writeFile } from "fs/promises";
import { config } from "dotenv";
import { fetchModelsWithCache } from "./fetch";
import { computeClassification, buildTodoList } from "./normalize";
import { fetchBitsWithModel, updateBitParameters, upsertLlmModel, computeTier, disconnect } from "./db";
import type { AAModel, ModelClassification, TodoEntry } from "./types";

const DRY_RUN = process.argv.includes("--dry-run");
const ROOT_DIR = resolve(import.meta.dir, "../..");

config({ path: resolve(ROOT_DIR, ".env") });

const apiKey = process.env.ARTIFICIAL_ANALYSIS;
if (!apiKey) {
	console.error("Missing ARTIFICIAL_ANALYSIS in .env");
	process.exit(1);
}

const dbUrl = process.env.DATABASE_URL;
if (!dbUrl) {
	console.error("Missing DATABASE_URL in .env");
	process.exit(1);
}

const TODO_PATH = resolve(import.meta.dir, "todo.json");
const CLASSIFICATION_KEYS: (keyof ModelClassification)[] = [
	"coding", "cost", "creativity", "factuality",
	"function_calling", "multilinguality", "openness",
	"reasoning", "safety", "speed",
];

// ANSI colors
const RED = "\x1b[31m";
const GREEN = "\x1b[32m";
const YELLOW = "\x1b[33m";
const CYAN = "\x1b[36m";
const DIM = "\x1b[2m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

function formatDiff(
	slug: string,
	modelName: string,
	bitId: string,
	oldClass: ModelClassification | null,
	newClass: ModelClassification,
	tierChange?: { oldTier: string; newTier: string } | null,
	releaseDate?: string | null,
): string {
	const lines: string[] = [];
	lines.push(`${BOLD}${CYAN}── ${modelName}${RESET} ${DIM}(${slug}) bit:${bitId}${RESET}`);

	if (!oldClass) {
		lines.push(`  ${GREEN}+ NEW classification${RESET}`);
		for (const key of CLASSIFICATION_KEYS) {
			lines.push(`    ${GREEN}${key}: ${newClass[key]}${RESET}`);
		}
		return lines.join("\n");
	}

	let hasChanges = false;
	for (const key of CLASSIFICATION_KEYS) {
		const oldVal = oldClass[key] ?? 0;
		const newVal = newClass[key];
		if (oldVal === newVal) {
			lines.push(`  ${DIM}  ${key}: ${oldVal}${RESET}`);
		} else {
			hasChanges = true;
			const arrow = newVal > oldVal ? "▲" : "▼";
			const color = newVal > oldVal ? GREEN : RED;
			const delta = (newVal - oldVal).toFixed(2);
			const sign = newVal > oldVal ? "+" : "";
			lines.push(`  ${color}${arrow} ${key}: ${oldVal} → ${newVal} (${sign}${delta})${RESET}`);
		}
	}

	if (tierChange && tierChange.oldTier !== tierChange.newTier) {
		hasChanges = true;
		lines.push(`  ${YELLOW}⚙ tier: ${tierChange.oldTier} → ${tierChange.newTier}${RESET}`);
	} else if (tierChange) {
		lines.push(`  ${DIM}  tier: ${tierChange.oldTier}${RESET}`);
	}

	if (releaseDate) {
		lines.push(`  ${DIM}  dates → ${releaseDate}${RESET}`);
	}

	if (!hasChanges) {
		return `${BOLD}${CYAN}── ${modelName}${RESET} ${DIM}(${slug}) bit:${bitId} — no changes${RESET}`;
	}

	return lines.join("\n");
}

async function loadExistingTodos(): Promise<Map<string, Partial<ModelClassification>>> {
	const overrides = new Map<string, Partial<ModelClassification>>();
	try {
		const raw = await readFile(TODO_PATH, "utf-8");
		const todos: TodoEntry[] = JSON.parse(raw);
		for (const entry of todos) {
			const partial: Partial<ModelClassification> = {};
			const manual = entry as unknown as Record<string, unknown>;
			for (const key of ["creativity", "multilinguality", "openness", "safety"] as const) {
				if (typeof manual[key] === "number" && (manual[key] as number) > 0) {
					partial[key] = manual[key] as number;
				}
			}
			if (Object.keys(partial).length > 0) {
				overrides.set(entry.modelSlug, partial);
			}
		}
	} catch {
		// no existing todo.json — that's fine
	}
	return overrides;
}

async function main() {
	if (DRY_RUN) {
		console.log(`${BOLD}${YELLOW}=== Model Evaluation Pipeline (DRY RUN) ===${RESET}\n`);
	} else {
		console.log("=== Model Evaluation Pipeline ===\n");
	}

	// Step 1: Fetch models (API with cache fallback)
	const { models, maxes } = await fetchModelsWithCache(apiKey!, ROOT_DIR);

	const modelBySlug = new Map<string, AAModel>();
	for (const m of models) {
		modelBySlug.set(m.slug, m);
	}

	const allPrices = models
		.map((m) => m.pricing.price_1m_blended_3_to_1)
		.filter((p): p is number => p != null && p > 0);

	const todoOverrides = await loadExistingTodos();

	// Step 3: Upsert LlmModel records (skip in dry run)
	if (!DRY_RUN) {
		console.log("\n[db] Upserting LlmModel records...");
		let upsertCount = 0;
		for (const aaModel of models) {
			await upsertLlmModel(aaModel);
			upsertCount++;
		}
		console.log(`[db] Upserted ${upsertCount} LlmModel records`);
	} else {
		console.log(`${DIM}[dry-run] Skipping LlmModel upserts (${models.length} models)${RESET}`);
	}

	// Step 4: Fetch bits and compute classifications
	console.log("\n[db] Fetching bits with modelSlug...");
	const bits = await fetchBitsWithModel();
	console.log(`[db] Found ${bits.length} bits with modelSlug`);

	const todoEntries: { slug: string; name: string; missing: string[] }[] = [];
	const seenSlugs = new Set<string>();
	let updatedCount = 0;
	let skippedCount = 0;
	let unchangedCount = 0;

	const dryRunChanges: {
		bitId: string;
		slug: string;
		modelName: string;
		old: ModelClassification | null;
		new: ModelClassification;
		tierChange?: { oldTier: string; newTier: string } | null;
	}[] = [];

	if (DRY_RUN) console.log(`\n${BOLD}── Changes ──${RESET}\n`);

	for (const bit of bits) {
		const slug = bit.modelSlug;
		if (!slug) continue;

		const aaModel = modelBySlug.get(slug);
		if (!aaModel) {
			console.warn(`[warn] No AA data for slug "${slug}" (bit ${bit.id})`);
			skippedCount++;
			continue;
		}

		const isLocal = Boolean(bit.downloadLink?.trim());
		const overrides = todoOverrides.get(slug);
		const oldClassification = (bit.parameters?.model_classification as ModelClassification | undefined) ?? null;

		const { classification, missingFields } = computeClassification(
			aaModel,
			maxes,
			allPrices,
			isLocal,
			oldClassification,
			overrides,
		);

		const tierResult = computeTier(bit.parameters, classification.cost);

		if (DRY_RUN) {
			const classUnchanged = oldClassification && CLASSIFICATION_KEYS.every(
				(k) => (oldClassification[k] ?? 0) === classification[k],
			);
			const tierUnchanged = !tierResult || tierResult.oldTier === tierResult.newTier;

			if (classUnchanged && tierUnchanged) {
				unchangedCount++;
			} else {
				dryRunChanges.push({
					bitId: bit.id,
					slug,
					modelName: aaModel.name,
					old: oldClassification,
					new: classification,
					tierChange: tierResult,
				});
				console.log(formatDiff(slug, aaModel.name, bit.id, oldClassification, classification, tierResult, aaModel.release_date));
				console.log();
			}
		} else {
			await updateBitParameters(bit.id, classification, bit.parameters, aaModel.release_date);
		}

		updatedCount++;

		if (!seenSlugs.has(slug)) {
			seenSlugs.add(slug);
			if (missingFields.length > 0) {
				todoEntries.push({ slug, name: aaModel.name, missing: missingFields });
			}
		}
	}

	if (DRY_RUN) {
		console.log(`${BOLD}── Summary ──${RESET}`);
		console.log(`  ${GREEN}Changed: ${dryRunChanges.length}${RESET}`);
		console.log(`  ${DIM}Unchanged: ${unchangedCount}${RESET}`);
		console.log(`  ${YELLOW}Skipped (no AA data): ${skippedCount}${RESET}`);

		// Write preview JSON
		const previewPath = resolve(ROOT_DIR, "tmp", "dry-run-preview.json");
		await writeFile(previewPath, JSON.stringify(dryRunChanges, null, 2));
		console.log(`\n  Preview written to ${DIM}tmp/dry-run-preview.json${RESET}`);
	} else {
		console.log(`[db] Updated ${updatedCount} bits, skipped ${skippedCount}`);
	}

	// Write todo.json
	const todos = buildTodoList(todoEntries);
	if (!DRY_RUN) {
		await writeFile(TODO_PATH, JSON.stringify(todos, null, 2));
		console.log(`\n[todo] Wrote ${todos.length} entries to todo.json`);
	} else {
		console.log(`\n${DIM}[dry-run] Would write ${todos.length} todo entries${RESET}`);
	}

	await disconnect();
	console.log(DRY_RUN ? `\n${BOLD}${YELLOW}=== Dry run complete — no DB writes ===${RESET}` : "\n=== Done ===");
}

main().catch((err) => {
	console.error("Fatal error:", err);
	disconnect().catch(() => {});
	process.exit(1);
});
