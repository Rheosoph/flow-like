import type { INode } from "../../schema/flow/board";
import type { CatalogEntry } from "../types";

function inlineSchemaTitle(
	schema: string | null | undefined,
	memo: Map<string, string>,
): string {
	if (!schema || schema[0] !== "{") return "";
	const cached = memo.get(schema);
	if (cached !== undefined) return cached;
	let title = "";
	try {
		const parsed: unknown = JSON.parse(schema);
		if (parsed && typeof parsed === "object" && "title" in parsed) {
			const value = (parsed as { title: unknown }).title;
			if (typeof value === "string") title = value;
		}
	} catch {}
	memo.set(schema, title);
	return title;
}

/** Projects catalog nodes onto what the neural model featurizes. Kept free of model code for the main thread. */
export function toCatalogEntries(nodes: INode[]): CatalogEntry[] {
	const memo = new Map<string, string>();
	return nodes.map((node) => ({
		name: node.name,
		friendlyName: node.friendly_name ?? "",
		category: node.category ?? "",
		description: node.description ?? "",
		pins: Object.values(node.pins ?? {})
			.sort((a, b) => (a.index ?? 0) - (b.index ?? 0))
			.map((pin) => ({
				name: pin.name,
				pinType: String(pin.pin_type) === "Output" ? "Output" : "Input",
				dataType: String(pin.data_type),
				valueType: String(pin.value_type),
				schema: inlineSchemaTitle(pin.schema, memo),
			})),
	}));
}
