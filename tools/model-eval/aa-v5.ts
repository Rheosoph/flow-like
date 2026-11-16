export const AA_V5_VERSION = "5.0.1" as const;
export const AA_MODELS_URL = "https://artificialanalysis.ai/models";

type JsonObject = Record<string, unknown>;
type Measurement = number | null;

/** Each value uses the source benchmark's units, including Elo for Briefcase. */
export interface AAV5Evaluations {
	briefcaseElo?: Measurement;
	automationBenchPartialScore?: Measurement;
	terminalBench40?: Measurement;
	frontierCode20?: Measurement;
	omniscience?: Measurement;
	gdpPdfAllPass?: Measurement;
	hleDiamond?: Measurement;
	critptAgenticHard?: Measurement;
	terminalBenchScience?: Measurement;
}

export interface AAV5Model {
	id: string;
	slug: string;
	name: string;
	release: { slug: string; name: string };
	effort?: { slug: string; label?: string; level?: number } | null;
	isReasoning: boolean;
	releaseDate: string | null;
	creator: { slug: string; name: string };
	intelligenceIndex?: Measurement;
	evaluations: AAV5Evaluations;
	omniscienceAccuracy?: Measurement;
	omniscienceHallucinationRate?: Measurement;
	version: typeof AA_V5_VERSION;
	sourceUrl: string;
	fetchedAt: string;
}

export interface AAV5Source {
	sourceUrl: string;
	fetchedAt: string;
}

export interface AAV5Page extends AAV5Source {
	version: typeof AA_V5_VERSION;
	models: AAV5Model[];
}

export interface AAV5Catalog {
	version: typeof AA_V5_VERSION;
	models: AAV5Model[];
	pages: Array<AAV5Source & { status: number; modelCount: number }>;
	missingSlugs: string[];
}

export interface AAV5FetchOptions {
	fetcher?: (url: string, init?: RequestInit) => Promise<Response>;
	now?: () => Date;
	concurrency?: number;
	timeoutMs?: number;
}

function object(value: unknown): JsonObject | undefined {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as JsonObject)
		: undefined;
}

function string(value: unknown): value is string {
	return typeof value === "string" && value.length > 0;
}

function measurement(
	row: JsonObject,
	key: string,
	context: string,
): Measurement | undefined {
	if (!Object.hasOwn(row, key)) return undefined;
	const value = row[key];
	if (value === null) return null;
	if (typeof value === "number" && Number.isFinite(value)) return value;
	throw new Error(`Invalid AA measurement ${context}.${key}`);
}

function optionalMeasurement(
	row: JsonObject,
	key: string,
	context: string,
): Record<string, Measurement> {
	const value = measurement(row, key, context);
	return value === undefined ? {} : { [key]: value };
}

function visibleText(html: string): string {
	return html
		.replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, " ")
		.replace(/<[^>]+>/g, " ")
		.replace(/&nbsp;|&#160;|&#x0*a0;/gi, " ")
		.replace(/\s+/g, " ");
}

function verifyVersion(html: string): void {
	const versions = [
		...visibleText(html).matchAll(
			/Artificial Analysis Intelligence Index\s+v(\d+\.\d+(?:\.\d+)?)\s+incorporates\s+(\d+)\s+evaluations/g,
		),
	];
	if (versions.length === 0) {
		throw new Error("AA page does not identify its Intelligence Index version");
	}
	for (const match of versions) {
		if (match[1] !== AA_V5_VERSION || match[2] !== "9") {
			throw new Error(
				`Expected AA Intelligence Index v${AA_V5_VERSION} with 9 evaluations; found v${match[1]} with ${match[2]}`,
			);
		}
	}
}

function parseModel(
	row: JsonObject,
	source: AAV5Source,
): AAV5Model | undefined {
	const release = object(row.release);
	const creator = object(row.creator);
	// Navigation cards omit the variant's identity and model configuration.
	if (
		!string(row.id) ||
		!string(row.slug) ||
		!string(row.name) ||
		!release ||
		!string(release.slug) ||
		!string(release.name) ||
		!creator ||
		!string(creator.slug) ||
		!string(creator.name) ||
		typeof row.isReasoning !== "boolean" ||
		typeof row.isOpenWeights !== "boolean" ||
		!Object.hasOwn(row, "contextWindowTokens") ||
		!(row.releaseDate === null || string(row.releaseDate))
	) {
		return undefined;
	}

	let effort: Pick<AAV5Model, "effort"> = {};
	if (Object.hasOwn(row, "effort")) {
		const rawEffort = object(row.effort);
		if (row.effort === null) {
			effort = { effort: null };
		} else if (rawEffort && string(rawEffort.slug)) {
			effort = {
				effort: {
					slug: rawEffort.slug,
					...(string(rawEffort.label) ? { label: rawEffort.label } : {}),
					...(typeof rawEffort.level === "number" &&
					Number.isFinite(rawEffort.level)
						? { level: rawEffort.level }
						: {}),
				},
			};
		} else {
			throw new Error(`Invalid AA effort for ${row.slug}`);
		}
	}

	const evaluations: AAV5Evaluations = {};
	for (const key of [
		"automationBenchPartialScore",
		"terminalBench40",
		"frontierCode20",
		"omniscience",
		"gdpPdfAllPass",
		"hleDiamond",
		"critptAgenticHard",
		"terminalBenchScience",
	] as const) {
		const value = measurement(row, key, row.slug);
		if (value !== undefined) evaluations[key] = value;
	}
	if (Object.hasOwn(row, "briefcaseBreakdown")) {
		if (row.briefcaseBreakdown === null) {
			evaluations.briefcaseElo = null;
		} else {
			const breakdown = object(row.briefcaseBreakdown);
			if (!breakdown) throw new Error(`Invalid AA Briefcase for ${row.slug}`);
			if (breakdown.overall === null) {
				evaluations.briefcaseElo = null;
			} else if (Object.hasOwn(breakdown, "overall")) {
				const overall = object(breakdown.overall);
				if (!overall) throw new Error(`Invalid AA Briefcase for ${row.slug}`);
				const elo = measurement(overall, "elo", `${row.slug}.briefcase`);
				if (elo !== undefined) evaluations.briefcaseElo = elo;
			}
		}
	}
	return {
		id: row.id,
		slug: row.slug,
		name: row.name,
		release: { slug: release.slug, name: release.name },
		...effort,
		isReasoning: row.isReasoning,
		releaseDate: row.releaseDate,
		creator: { slug: creator.slug, name: creator.name },
		...optionalMeasurement(row, "intelligenceIndex", row.slug),
		evaluations,
		...optionalMeasurement(row, "omniscienceAccuracy", row.slug),
		...optionalMeasurement(row, "omniscienceHallucinationRate", row.slug),
		version: AA_V5_VERSION,
		...source,
	};
}

/** Decode JSON from React Flight scripts. No script from the page is executed. */
export function parseAAV5Page(html: string, source: AAV5Source): AAV5Page {
	verifyVersion(html);
	if (!source.sourceUrl || !Number.isFinite(Date.parse(source.fetchedAt))) {
		throw new Error("AA source URL and fetchedAt timestamp are required");
	}
	const chunks: string[] = [];
	for (const script of html.matchAll(
		/<script\b[^>]*>([\s\S]*?)<\/script\s*>/gi,
	)) {
		const push = /^\s*self\.__next_f\.push\(([\s\S]*)\)\s*;?\s*$/.exec(
			script[1],
		);
		if (!push) continue;
		try {
			const chunk: unknown = JSON.parse(push[1]);
			if (
				Array.isArray(chunk) &&
				chunk[0] === 1 &&
				typeof chunk[1] === "string"
			) {
				chunks.push(chunk[1]);
			}
		} catch {
			// Other scripts and JavaScript expressions are never interpreted.
		}
	}
	const models = new Map<string, AAV5Model>();
	const visit = (value: unknown): void => {
		if (Array.isArray(value)) {
			for (const child of value) visit(child);
			return;
		}
		const row = object(value);
		if (!row) return;
		const model = parseModel(row, source);
		if (model) {
			const previous = models.get(model.slug);
			if (previous && JSON.stringify(previous) !== JSON.stringify(model)) {
				throw new Error(`Conflicting AA model variants for ${model.slug}`);
			}
			models.set(model.slug, model);
		}
		for (const child of Object.values(row)) visit(child);
	};
	// React can split a JSON record between script tags, including inside strings.
	for (const line of chunks.join("").split("\n")) {
		const record = /^[0-9a-f]+:([\[{].*)$/.exec(line);
		if (!record) continue;
		let value: unknown;
		try {
			value = JSON.parse(record[1]);
		} catch {
			continue;
		}
		visit(value);
	}
	if (models.size === 0)
		throw new Error("AA page contains no full model variants");
	return { ...source, version: AA_V5_VERSION, models: [...models.values()] };
}

/** Fetch only exact variants; an unsuccessful request never falls back to cached data. */
export async function fetchAAV5Catalog(
	requiredSlugs: readonly string[] = [],
	options: AAV5FetchOptions = {},
): Promise<AAV5Catalog> {
	const requested = [...new Set(requiredSlugs)];
	for (const slug of requested) {
		if (!/^[a-z0-9][a-z0-9.-]*$/.test(slug)) {
			throw new Error(`Invalid AA model slug: ${slug}`);
		}
	}
	const concurrency = options.concurrency ?? 3;
	if (!Number.isInteger(concurrency) || concurrency < 1) {
		throw new Error("AA fetch concurrency must be a positive integer");
	}
	const fetcher = options.fetcher ?? fetch;
	const now = options.now ?? (() => new Date());
	const fetchPage = async (url: string, allowMissing = false) => {
		const response = await fetcher(url, {
			headers: { Accept: "text/html" },
			signal: AbortSignal.timeout(options.timeoutMs ?? 30_000),
		});
		const source: AAV5Source = {
			sourceUrl: response.url || url,
			fetchedAt: now().toISOString(),
		};
		if (new URL(source.sourceUrl).origin !== new URL(AA_MODELS_URL).origin) {
			throw new Error(
				`AA request redirected to an unexpected origin: ${source.sourceUrl}`,
			);
		}
		if (allowMissing && response.status === 404) {
			return { source, status: 404, page: undefined };
		}
		if (!response.ok) {
			throw new Error(`AA request failed (${response.status}): ${url}`);
		}
		const page = parseAAV5Page(await response.text(), source);
		return { source, status: response.status, page };
	};
	const main = await fetchPage(AA_MODELS_URL);
	if (!main.page) throw new Error("AA main catalog is unavailable");
	const models = new Map(main.page.models.map((model) => [model.slug, model]));
	const missing = requested.filter((slug) => !models.has(slug));
	const pages: AAV5Catalog["pages"] = [
		{
			...main.source,
			status: main.status,
			modelCount: main.page.models.length,
		},
	];
	const fetched = new Map<string, Awaited<ReturnType<typeof fetchPage>>>();
	let next = 0;
	await Promise.all(
		Array.from({ length: Math.min(concurrency, missing.length) }, async () => {
			while (next < missing.length) {
				const slug = missing[next++];
				fetched.set(slug, await fetchPage(`${AA_MODELS_URL}/${slug}`, true));
			}
		}),
	);
	for (const slug of missing) {
		const result = fetched.get(slug);
		if (!result) throw new Error(`AA request did not complete for ${slug}`);
		pages.push({
			...result.source,
			status: result.status,
			modelCount: result.page?.models.length ?? 0,
		});
		const exact = result.page?.models.find((model) => model.slug === slug);
		if (exact) models.set(slug, exact);
	}
	return {
		version: AA_V5_VERSION,
		models: [...models.values()],
		pages,
		missingSlugs: requested.filter((slug) => !models.has(slug)),
	};
}
