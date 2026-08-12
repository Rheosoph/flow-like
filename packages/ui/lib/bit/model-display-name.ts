import type { IBit } from "../schema";

const HEX_ID_RE = /^[0-9a-f]{16,}$/i;
const UUID_RE =
	/^[0-9a-f]{8}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{12}$/i;
/** cuid2 — the id format every Bit created through the app or hub admin gets. */
const CUID2_RE = /^[a-z][a-z0-9]{23}$/;

/**
 * True for model strings that are an internal Bit / deployment id rather than a
 * name a user would recognise. Providers fall back to the Bit id whenever the
 * model definition carries no explicit `model_id`, which is how these end up in
 * usage stats and trace views.
 */
export function isBitIdLike(value: string): boolean {
	return HEX_ID_RE.test(value) || UUID_RE.test(value) || CUID2_RE.test(value);
}

/** Human-readable name of a model Bit, preferring its catalog metadata. */
export function bitModelName(bit: IBit | undefined | null): string | undefined {
	if (!bit) return undefined;
	const meta = bit.meta?.en ?? Object.values(bit.meta ?? {})[0];
	const candidates = [
		meta?.name,
		bit.model_evaluation?.name,
		bit.model_slug,
		bit.parameters?.provider?.model_id,
	];
	for (const candidate of candidates) {
		const name = typeof candidate === "string" ? candidate.trim() : "";
		if (name && !isBitIdLike(name)) return name;
	}
	return undefined;
}

export interface IModelLabel {
	/** Text to render for this model. */
	label: string;
	/** The label came from a resolved Bit instead of the raw model string. */
	resolved: boolean;
	/** The raw model string is an opaque id that could not be resolved. */
	opaque: boolean;
}

/**
 * Turns a raw model string into something displayable: a resolved Bit name when
 * one is known, the bare model name for provider-qualified strings
 * (`openai/gpt-4o`), or a shortened id as a last resort.
 */
export function modelLabel(
	model: string | undefined | null,
	names?: ReadonlyMap<string, string>,
): IModelLabel {
	const raw = model?.trim() ?? "";
	if (!raw || raw === "unknown") {
		return { label: "Unknown Model", resolved: false, opaque: false };
	}

	const resolved = names?.get(raw);
	if (resolved) return { label: resolved, resolved: true, opaque: false };

	if (isBitIdLike(raw)) {
		return { label: `${raw.slice(0, 8)}…`, resolved: false, opaque: true };
	}

	const parts = raw.split("/");
	return {
		label: parts[parts.length - 1] || raw,
		resolved: false,
		opaque: false,
	};
}
