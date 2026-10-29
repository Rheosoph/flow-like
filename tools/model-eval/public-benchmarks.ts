import type { AAOpenSourceCategorization } from "./types";

type JsonObject = Record<string, unknown>;

export interface PublicModelBenchmarks {
	opennessIndex?: number;
	openSourceCategorization?: AAOpenSourceCategorization;
	omniscienceAccuracy?: number;
	omniscienceHallucinationRate?: number;
	automationBench?: number;
	multilingualIndex?: number;
}

const CATEGORIES: Record<string, AAOpenSourceCategorization> = {
	proprietary: "Proprietary",
	permissive: "Open Weights (Permissive License)",
	"commercial-license": "Open Weights (License Required for Commercial Use)",
};

function object(value: unknown): JsonObject | undefined {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as JsonObject)
		: undefined;
}

function number(value: unknown, max = 1): number | undefined {
	return typeof value === "number" &&
		Number.isFinite(value) &&
		value >= 0 &&
		value <= max
		? value
		: undefined;
}

/** Read serialized page data without executing scripts from the benchmark site. */
export function parsePublicModelBenchmarks(
	html: string,
): Map<string, PublicModelBenchmarks> {
	const chunks: string[] = [];
	for (const match of html.matchAll(
		/self\.__next_f\.push\((\[[\s\S]*?\])\)<\/script>/g,
	)) {
		try {
			const chunk: unknown = JSON.parse(match[1]);
			if (
				Array.isArray(chunk) &&
				chunk[0] === 1 &&
				typeof chunk[1] === "string"
			) {
				chunks.push(chunk[1]);
			}
		} catch {
			// Ignore non-JSON script contents.
		}
	}
	const result = new Map<string, PublicModelBenchmarks>();
	const visit = (value: unknown) => {
		if (Array.isArray(value)) {
			for (const child of value) visit(child);
			return;
		}
		const row = object(value);
		if (!row) return;
		if (typeof row.slug === "string" && typeof row.name === "string") {
			const openness = object(row.openness);
			const omniscience = object(row.omniscienceBreakdown);
			const multilingual = object(row.multilingual);
			const category =
				typeof row.openSourceCategorization === "string"
					? CATEGORIES[row.openSourceCategorization]
					: undefined;
			const parsed: PublicModelBenchmarks = {
				opennessIndex: number(
					openness?.opennessIndex ?? row.opennessIndex,
					100,
				),
				openSourceCategorization: category,
				omniscienceAccuracy: number(
					row.omniscienceAccuracy ?? omniscience?.accuracy,
				),
				omniscienceHallucinationRate: number(
					row.omniscienceHallucinationRate ?? omniscience?.hallucinationRate,
				),
				automationBench: number(row.automationBenchPartialScore),
				multilingualIndex: number(multilingual?.average),
			};
			const present = Object.fromEntries(
				Object.entries(parsed).filter(([, entry]) => entry !== undefined),
			);
			if (Object.keys(present).length > 0) {
				result.set(row.slug, { ...result.get(row.slug), ...present });
			}
		}
		for (const child of Object.values(row)) visit(child);
	};
	// A streamed record can span several script tags. Reassemble before decoding.
	for (const line of chunks.join("").split("\n")) {
		const record = /^[0-9a-f]+:([\[{].*)$/.exec(line);
		if (!record) continue;
		try {
			visit(JSON.parse(record[1]));
		} catch {
			// React module references and partial records do not carry model rows.
		}
	}
	return result;
}
