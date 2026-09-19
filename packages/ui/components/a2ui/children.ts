import type { Children, DataScope, SurfaceComponent } from "./types";

/** Every component id this component renders: child list, named slots, template and tab/accordion/overlay/popover content. */
export function getComponentChildren(component?: SurfaceComponent): string[] {
	const props = component?.component as unknown as
		| Record<string, unknown>
		| undefined;
	if (!props) return [];
	const childList = component?.component.children;
	const children = [
		...(childList && "explicitList" in childList ? childList.explicitList : []),
		...[props.child, props.entryPointChild, props.contentChild].filter(
			(id): id is string => typeof id === "string",
		),
	];
	if (childList && "template" in childList)
		children.push(childList.template.templateComponentId);
	if (component?.component.type === "tabs")
		children.push(
			...(component.component.tabs ?? []).map((tab) => tab.contentComponentId),
		);
	if (component?.component.type === "accordion")
		children.push(
			...(component.component.items ?? []).map(
				(item) => item.contentComponentId,
			),
		);
	if (component?.component.type === "overlay") {
		children.push(
			component.component.baseComponentId,
			...(component.component.overlays ?? []).map(
				(overlay) => overlay.componentId,
			),
		);
	}
	if (component?.component.type === "popover")
		children.push(component.component.contentComponentId);
	return children.filter(
		(id): id is string => typeof id === "string" && id.length > 0,
	);
}

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
