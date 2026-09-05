import type { A2UIComponent, SurfaceComponent } from "../a2ui/types";

export function getExplicitChildren(component?: SurfaceComponent): string[] {
	const children = component?.component?.children;
	return children && "explicitList" in children ? children.explicitList : [];
}

const CHILD_LIST_CONTAINERS = new Set([
	"row",
	"column",
	"stack",
	"grid",
	"card",
	"scrollArea",
	"modal",
	"drawer",
	"tooltip",
	"popover",
	"box",
	"center",
	"absolute",
	"aspectRatio",
]);

export function canAcceptComponentChildren(
	component?: SurfaceComponent,
): boolean {
	if (
		!component?.component ||
		!CHILD_LIST_CONTAINERS.has(component.component.type)
	)
		return false;
	const children = component.component.children;
	return !children || "explicitList" in children;
}

export function getComponentChildren(component?: SurfaceComponent): string[] {
	const props = component?.component as unknown as
		| Record<string, unknown>
		| undefined;
	if (!props) return [];
	const children = [
		...getExplicitChildren(component),
		...[props.child, props.entryPointChild, props.contentChild].filter(
			(id): id is string => typeof id === "string",
		),
	];
	const childTemplate = component?.component.children;
	if (childTemplate && "template" in childTemplate)
		children.push(childTemplate.template.templateComponentId);
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

export function findComponentParent(
	components: Map<string, SurfaceComponent>,
	childId: string,
): string | null {
	for (const [id, component] of components) {
		if (getComponentChildren(component).includes(childId)) return id;
	}
	return null;
}

export function canReorderComponent(
	components: Map<string, SurfaceComponent>,
	id: string,
): boolean {
	const parentId = findComponentParent(components, id);
	if (!parentId) return false;
	const parent = components.get(parentId);
	if (getExplicitChildren(parent).includes(id)) return true;
	const props = parent?.component as unknown as
		| Record<string, unknown>
		| undefined;
	// Named tab, accordion and overlay content belongs to its slot. Moving that
	// reference would leave the slot invalid, so only its child list is editable.
	return (
		!!props &&
		[props.child, props.entryPointChild, props.contentChild].includes(id)
	);
}

export function canMoveComponent(
	components: Map<string, SurfaceComponent>,
	id: string,
	parentId: string,
): boolean {
	if (
		!components.has(id) ||
		!canAcceptComponentChildren(components.get(parentId))
	)
		return false;
	// A root has no parent to detach from. Keep it in place.
	if (!canReorderComponent(components, id)) return false;
	const pending = [id];
	const visited = new Set<string>();
	while (pending.length) {
		const current = pending.pop();
		if (current === undefined) continue;
		if (current === parentId) return false;
		if (visited.has(current)) continue;
		visited.add(current);
		pending.push(...getComponentChildren(components.get(current)));
	}
	return true;
}

/** The insertion index refers to the child list before removing the moving item. */
export function moveComponentInTree(
	components: Map<string, SurfaceComponent>,
	id: string,
	parentId: string,
	index?: number,
): Map<string, SurfaceComponent> {
	if (!canMoveComponent(components, id, parentId)) return components;
	const fromParentId = findComponentParent(components, id);
	if (fromParentId === null) return components;
	const fromParent = components.get(fromParentId);
	const parent = components.get(parentId);
	if (!fromParent || !parent) return components;
	const children = [...getExplicitChildren(parent)];
	const oldIndex = children.indexOf(id);
	const targetIndex = Math.max(
		0,
		Math.min(index ?? children.length, children.length),
	);
	const insertionIndex =
		oldIndex >= 0 && targetIndex > oldIndex ? targetIndex - 1 : targetIndex;
	const reordered = children.filter((childId) => childId !== id);
	reordered.splice(insertionIndex, 0, id);
	if (
		fromParentId === parentId &&
		reordered.every((childId, i) => childId === children[i])
	) {
		return components;
	}
	const next = new Map(components);
	if (fromParentId !== parentId) {
		const props = { ...fromParent.component } as unknown as Record<
			string,
			unknown
		>;
		if (getExplicitChildren(fromParent).includes(id)) {
			props.children = {
				explicitList: getExplicitChildren(fromParent).filter(
					(childId) => childId !== id,
				),
			};
		}
		for (const slot of ["child", "entryPointChild", "contentChild"]) {
			if (props[slot] === id) delete props[slot];
		}
		next.set(fromParentId, {
			...fromParent,
			component: props as unknown as A2UIComponent,
		});
	}
	next.set(parentId, {
		...parent,
		component: {
			...parent.component,
			children: { explicitList: reordered },
		} as A2UIComponent,
	});
	return next;
}
