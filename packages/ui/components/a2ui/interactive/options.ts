import type { SelectOption } from "../types";

function asText(value: unknown): string | undefined {
	if (typeof value === "string") return value;
	if (typeof value === "number" || typeof value === "boolean")
		return String(value);
	return undefined;
}

function toOption(entry: unknown): SelectOption | undefined {
	const direct = asText(entry);
	if (direct !== undefined)
		return direct === "" ? undefined : { value: direct, label: direct };

	if (typeof entry !== "object" || entry === null) return undefined;
	const { value, label } = entry as { value?: unknown; label?: unknown };
	const optionValue = asText(value);
	if (optionValue === undefined || optionValue === "") return undefined;
	const optionLabel = asText(label);
	return {
		value: optionValue,
		label:
			optionLabel === undefined || optionLabel === ""
				? optionValue
				: optionLabel,
	};
}

/**
 * Radix reserves the empty string for "no selection", so an option carrying one throws
 * and takes the whole surface — builder canvas included — down with it. Option lists come
 * from page authors clearing a value mid-edit, from flows, and from data bindings, so they
 * arrive as bare strings, numbers, objects with a missing label, or repeated values. Every
 * list is normalized here before it reaches an item.
 */
export function normalizeOptions(raw: unknown): SelectOption[] {
	if (!Array.isArray(raw)) return [];
	const seen = new Set<string>();
	const options: SelectOption[] = [];
	for (const entry of raw) {
		const option = toOption(entry);
		if (!option || seen.has(option.value)) continue;
		seen.add(option.value);
		options.push(option);
	}
	return options;
}

/** The rendered selection, as the string an option value was normalized to. */
export function toOptionValue(value: unknown): string {
	return asText(value) ?? "";
}
