import type { Children, DataScope } from "./types";

export interface ResolvedChild {
	id: string;
	key: string;
	scope?: DataScope;
}

function getItemId(
	item: unknown,
	itemIdPath: string | undefined,
): string | undefined {
	if (!itemIdPath || item === null || typeof item !== "object")
		return undefined;

	const parts = itemIdPath
		.replace(/^\$\./, "")
		.replace(/^\//, "")
		.split(/[./]/)
		.filter(Boolean);

	let current: unknown = item;
	for (const part of parts) {
		if (current === null || typeof current !== "object") return undefined;
		current = (current as Record<string, unknown>)[part];
	}

	if (typeof current === "string" || typeof current === "number") {
		return String(current);
	}

	return undefined;
}

export function resolveChildSpecs(
	children: Children | undefined,
	resolve: (boundValue: { path: string }) => unknown,
): ResolvedChild[] {
	if (!children) return [];

	if ("explicitList" in children) {
		return children.explicitList.map((id) => ({ id, key: id }));
	}

	if ("template" in children) {
		const { template } = children;
		const items = resolve({ path: template.dataPath });
		if (!Array.isArray(items)) return [];

		return items.map((item, index) => {
			const itemId = getItemId(item, template.itemIdPath);
			const key = `${template.templateComponentId}:${itemId ?? index}`;
			return {
				id: template.templateComponentId,
				key,
				scope: {
					dataPath: template.dataPath,
					index,
					item,
					itemId,
				},
			};
		});
	}

	return [];
}
