import { AA_V5_VERSION } from "./aa-v5";

type Evaluation = Record<string, unknown>;

const METHODOLOGY =
	"https://artificialanalysis.ai/methodology/intelligence-benchmarking";
const CURRENT_SUMMARY =
	/^(?:Artificial Analysis Intelligence Index v[\d.]+:|Estimated Intelligence Index on the AA v[\d.]+ scale:|No current Artificial Analysis Intelligence Index v[\d.]+ result)/;
const AA_FIELDS = new Set([
	"coding",
	"reasoning",
	"factuality",
	"function calling",
	"tool use",
	"safety",
]);
const CAPABILITY_SCOPE =
	"Coding, reasoning, tool-use and factuality ratings use available AA v5.0.1 task results. Unavailable measurements are left unrated. The AA intelligence suite does not measure harmful-content safety.";

function record(value: unknown): Evaluation {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Evaluation)
		: {};
}

function score(value: unknown): value is number {
	return (
		typeof value === "number" &&
		Number.isFinite(value) &&
		value >= 0 &&
		value <= 100
	);
}

function number(value: number): string {
	return Number(value.toFixed(2)).toString();
}

function source(value: unknown): string | null {
	if (typeof value !== "string") return null;
	try {
		const url = new URL(value);
		return url.protocol === "https:" && !url.username && !url.password
			? url.href
			: null;
	} catch {
		return null;
	}
}

function summary(evaluations: Evaluation): string | null {
	if (
		evaluations.artificial_analysis_intelligence_index_version !== AA_V5_VERSION
	)
		return null;
	const measured = evaluations.artificial_analysis_intelligence_index;
	const benchmarkSource =
		source(evaluations.benchmark_source_url) ?? METHODOLOGY;
	if (evaluations.benchmark_status === "measured" && score(measured)) {
		const name =
			typeof evaluations.benchmark_model_name === "string"
				? evaluations.benchmark_model_name.trim()
				: "";
		const effort =
			typeof evaluations.benchmark_reasoning_effort === "string"
				? evaluations.benchmark_reasoning_effort.trim()
				: "";
		return `Artificial Analysis Intelligence Index v${AA_V5_VERSION}: ${number(measured)}${name ? ` for ${name}` : ""}. ${effort ? `Evaluated reasoning effort: ${effort}. ` : ""}Runtime settings can differ from the evaluated configuration. Source: ${benchmarkSource}`;
	}
	const estimate = record(evaluations.intelligence_estimate);
	if (
		evaluations.benchmark_status === "estimated" &&
		estimate.version === AA_V5_VERSION &&
		estimate.method === "paired-version-calibration" &&
		score(estimate.score) &&
		score(estimate.low) &&
		score(estimate.high) &&
		estimate.low <= estimate.score &&
		estimate.score <= estimate.high &&
		["low", "medium"].includes(String(estimate.confidence))
	) {
		const priorSource = source(record(estimate.prior).source);
		const links = [
			...new Set(
				[priorSource, benchmarkSource, METHODOLOGY].filter(
					(url): url is string => Boolean(url),
				),
			),
		];
		return `Estimated Intelligence Index on the AA v${AA_V5_VERSION} scale: ${number(estimate.score)}, judgment range ${number(estimate.low)} to ${number(estimate.high)}, ${estimate.confidence} confidence. This catalog estimate is calibrated from paired legacy-catalog and v${AA_V5_VERSION} scores; it is not a measured Artificial Analysis result. The legacy catalog mixed index versions, so this is not an exact v4.3 conversion. The range is not a statistical confidence interval. Sources: ${links.join(", ")}`;
	}
	if (
		["not_evaluated", "unmatched"].includes(
			String(evaluations.benchmark_status),
		)
	) {
		return `No current Artificial Analysis Intelligence Index v${AA_V5_VERSION} result${evaluations.benchmark_status === "unmatched" ? "; no matching evaluated model was found" : " is available"}. Source: ${benchmarkSource}`;
	}
	return null;
}

function rankingRationale(paragraph: string): string {
	const rationale = paragraph.replace(
		/^Ranking estimates:\s*Estimated intelligence [\s\S]*?The range is not a statistical confidence interval\.\s*/i,
		"",
	);
	// These sentences describe old score anchors, rather than the model's use or limitations.
	return rationale
		.split(/(?<=[.!?])\s+(?=[A-Z])/)
		.flatMap((sentence) => {
			if (
				/^(?:Estimated (?:on the current catalog intelligence scale|near|above)|Those models provide current index anchors|Base-model AA results span)\b/.test(
					sentence,
				)
			)
				return [];
			if (/^Close proxy to AA /.test(sentence))
				return [sentence.replace(/\s*\(\d+(?:\.\d+)?\)/g, "")];
			if (/^The \d+(?:\.\d+)? estimate allows for /.test(sentence))
				return [
					sentence.replace(/^The \d+(?:\.\d+)? estimate/, "The estimate"),
				];
			if (
				/^A modest uplift gives \d+(?:\.\d+)?, with uncertainty for /.test(
					sentence,
				)
			)
				return [
					sentence.replace(
						/^A modest uplift gives \d+(?:\.\d+)?, with uncertainty for /,
						"The estimate includes uncertainty for ",
					),
				];
			return [sentence];
		})
		.join(" ");
}

function capabilityProse(paragraph: string): string {
	if (paragraph.startsWith("Capability ratings:")) {
		let cleaned = paragraph.replace(
			/Ratings for ([^.]+) include researched estimates\./,
			(_, fields: string) => {
				const retained = fields
					.split(",")
					.map((field) => field.trim())
					.filter((field) => !AA_FIELDS.has(field));
				return retained.length
					? `Ratings for ${retained.join(", ")} include researched estimates.`
					: "";
			},
		);
		cleaned = cleaned.replace(
			/The Safety rating follows the catalog's factual-reliability convention\.\s*It is not a harmful-content safety assessment\.\s*/g,
			"",
		);
		return cleaned.includes(CAPABILITY_SCOPE)
			? cleaned
			: `${cleaned.trim()} ${CAPABILITY_SCOPE}`;
	}
	if (
		/^(?:Estimated capability evidence|AA-derived capability (?:ratings|scores))\b/.test(
			paragraph,
		)
	) {
		const lines = paragraph.split("\n").filter((line) => {
			const field = line.match(/^\s*[-*]\s*([\w ]+):\s*-?\d/);
			return !field || !AA_FIELDS.has(field[1].trim().replaceAll("_", " "));
		});
		return lines.length > 1 ? lines.join("\n") : "";
	}
	return paragraph;
}

/** Refresh benchmark prose while retaining runtime instructions and model-specific caveats. */
export function refreshBenchmarkDescription(
	description: string,
	evaluations: Evaluation,
): string {
	const current = summary(evaluations);
	if (!current) return description;
	const output: string[] = [];
	let inserted = false;
	const insert = () => {
		if (!inserted) output.push(current);
		inserted = true;
	};
	for (const paragraph of description.trim().split(/\n\s*\n/)) {
		if (CURRENT_SUMMARY.test(paragraph)) {
			insert();
			continue;
		}
		if (paragraph.startsWith("Ranking estimates:")) {
			insert();
			const rationale = rankingRationale(paragraph);
			if (rationale) output.push(rationale);
			continue;
		}
		if (/^Source:\s*https:\/\/artificialanalysis\.ai\/models\//.test(paragraph))
			continue;
		let cleaned = paragraph
			.replace(
				/Artificial Analysis reports an Intelligence Index of [\s\S]*?\.(?=\s|$)/g,
				"",
			)
			.replace(
				/The score describes that benchmark configuration; workflow reasoning settings can differ\.\s*/g,
				"",
			)
			.replace(
				/Current Artificial Analysis v[\d.]+ intelligence is unavailable; the catalog labels its inferred score as an estimate and preserves the earlier index as a historical benchmark\.\s*/g,
				"",
			)
			.replace(
				/The intelligence score is Artificial Analysis v[\d.]+ at [^.]+ reasoning effort\.\s*/g,
				"",
			)
			.replace(
				/Missing capability measurements are recorded as estimates with sources\./g,
				"Unavailable capability measurements are left unrated.",
			);
		if (
			evaluations.benchmark_status === "estimated" &&
			/modified model|uncensored artifact/.test(
				String(record(record(evaluations.intelligence_estimate).prior).basis),
			)
		) {
			cleaned = cleaned.replace(
				"Identical capability to the standard build, including image understanding.",
				"Based on the standard model, with image input support. Its intelligence estimate uses the base model as a proxy; this modified build has no direct AA v5.0.1 measurement.",
			);
		}
		const removedScore =
			cleaned !== paragraph &&
			/Artificial Analysis|intelligence score/.test(paragraph);
		cleaned = capabilityProse(cleaned.trim());
		if (removedScore && paragraph.startsWith("Artificial Analysis reports"))
			insert();
		if (cleaned) output.push(cleaned);
		if (removedScore) insert();
	}
	insert();
	return output.join("\n\n");
}
