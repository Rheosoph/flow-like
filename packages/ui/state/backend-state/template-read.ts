import type { IMetadata } from "../../lib";

export type TemplateMetadataEntry = [string, string, IMetadata | undefined];

export interface TemplateReadCoverage {
	complete: boolean;
	scope: string;
	warning?: string;
}

export interface TemplateReadOptions {
	/** Propagate failures instead of treating them as empty results. */
	strict?: boolean;
	/** Read metadata without downloading templates or updating caches. */
	readOnly?: boolean;
	onCoverage?: (coverage: TemplateReadCoverage) => void;
}

export const MAX_OWNED_TEMPLATE_METADATA = 1_000;

export class TemplateMetadataReadError extends Error {
	constructor(
		message: string,
		public readonly partialTemplates: TemplateMetadataEntry[],
	) {
		super(message);
		this.name = "TemplateMetadataReadError";
	}
}

export function templateReadErrorMessage(error: unknown): string {
	return (error instanceof Error ? error.message : String(error)).slice(0, 500);
}

/** Merge a bounded metadata snapshot. Remote metadata replaces matching local metadata. */
export async function readTemplateMetadataSnapshot(
	reads: { label: string; read: () => Promise<TemplateMetadataEntry[]> }[],
	options: TemplateReadOptions,
	coverage: TemplateReadCoverage,
): Promise<TemplateMetadataEntry[]> {
	const results = await Promise.allSettled(
		reads.map(({ read }) => Promise.resolve().then(read)),
	);
	const merged = new Map<string, TemplateMetadataEntry>();
	const errors: string[] = [];
	let capped = false;
	for (const [index, result] of results.entries()) {
		if (result.status === "rejected") {
			errors.push(
				`${reads[index].label}: ${templateReadErrorMessage(result.reason)}`,
			);
			continue;
		}
		if (!Array.isArray(result.value)) {
			errors.push(
				`${reads[index].label}: returned ${typeof result.value}, not a list`,
			);
			continue;
		}
		if (result.value.length > MAX_OWNED_TEMPLATE_METADATA) capped = true;
		for (const entry of result.value.slice(0, MAX_OWNED_TEMPLATE_METADATA)) {
			if (!Array.isArray(entry)) continue;
			const key = JSON.stringify([entry[0], entry[1]]);
			if (!merged.has(key) && merged.size >= MAX_OWNED_TEMPLATE_METADATA) {
				capped = true;
				continue;
			}
			merged.set(key, entry);
		}
	}
	options.onCoverage?.({
		...coverage,
		complete: coverage.complete && !capped && errors.length === 0,
		warning:
			[
				coverage.warning,
				capped
					? `Retained at most ${MAX_OWNED_TEMPLATE_METADATA} metadata entries.`
					: undefined,
			]
				.filter(Boolean)
				.join(" ") || undefined,
	});
	const entries = [...merged.values()].sort((left, right) => {
		const a = JSON.stringify([left[0], left[1]]);
		const b = JSON.stringify([right[0], right[1]]);
		return a < b ? -1 : a > b ? 1 : 0;
	});
	if (errors.length > 0 && options.strict) {
		throw new TemplateMetadataReadError(errors.join("; "), entries);
	}
	return entries;
}
