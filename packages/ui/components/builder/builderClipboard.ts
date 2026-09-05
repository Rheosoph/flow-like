import { createId } from "@paralleldrive/cuid2";
import type { IWidgetRef } from "../../state/backend-state/page-state";
import type { A2UIComponent, SurfaceComponent } from "../a2ui/types";
import {
	canAcceptComponentChildren,
	canMoveComponent,
	canReorderComponent,
	findComponentParent,
	getComponentChildren,
	getExplicitChildren,
	moveComponentInTree,
} from "./componentTree";

export interface BuilderClipboard {
	components: SurfaceComponent[];
	cut: boolean;
	rootIds: string[];
	widgetRefs?: Record<string, IWidgetRef>;
	sourceId?: string;
}

export function collectClipboard(
	components: Map<string, SurfaceComponent>,
	widgetRefs: Map<string, IWidgetRef>,
	ids: string[],
	cut: boolean,
	sourceId: string,
): BuilderClipboard | null {
	const selected = [...new Set(ids)].filter((id) => components.has(id));
	const descendants = new Set<string>();
	for (const id of selected) {
		const pending = [...getComponentChildren(components.get(id))];
		while (pending.length) {
			const childId = pending.pop();
			if (childId === undefined) continue;
			if (descendants.has(childId)) continue;
			descendants.add(childId);
			pending.push(...getComponentChildren(components.get(childId)));
		}
	}
	const rootIds = selected.filter(
		(id) =>
			!descendants.has(id) && (!cut || canReorderComponent(components, id)),
	);
	if (!rootIds.length) return null;
	const collected = new Map<string, SurfaceComponent>();
	const refs: Record<string, IWidgetRef> = {};
	const pending = [...rootIds].reverse();
	while (pending.length) {
		const id = pending.pop();
		if (id === undefined) continue;
		const component = components.get(id);
		if (!component || collected.has(id)) continue;
		collected.set(id, component);
		pending.push(...getComponentChildren(component).reverse());
		if (component.component.type === "widgetInstance") {
			const { instanceId } = component.component;
			const widget = widgetRefs.get(instanceId);
			if (widget) refs[instanceId] = widget;
		}
	}
	return structuredClone({
		components: [...collected.values()],
		cut,
		rootIds,
		widgetRefs: refs,
		sourceId,
	});
}

function resolvePasteParent(
	components: Map<string, SurfaceComponent>,
	selectionIds: string[],
	parentId?: string,
): string | undefined {
	let candidate: string | undefined = parentId ?? selectionIds[0];
	const visited = new Set<string>();
	while (candidate && !visited.has(candidate)) {
		if (canAcceptComponentChildren(components.get(candidate))) return candidate;
		visited.add(candidate);
		candidate = findComponentParent(components, candidate) ?? undefined;
	}
	if (parentId !== undefined) return undefined;
	return [...components.keys()].find(
		(id) =>
			!findComponentParent(components, id) &&
			canAcceptComponentChildren(components.get(id)),
	);
}

const COMPONENT_REFERENCE_KEYS = new Set([
	"child",
	"entryPointChild",
	"contentChild",
	"contentComponentId",
	"baseComponentId",
	"componentId",
	"templateComponentId",
	"sourceComponentId",
	"targetComponentId",
]);

function remapReferences(value: unknown, ids: Map<string, string>): unknown {
	if (Array.isArray(value))
		return value.map((entry) => remapReferences(entry, ids));
	if (!value || typeof value !== "object") return value;
	return Object.fromEntries(
		Object.entries(value).map(([key, entry]) => [
			key,
			COMPONENT_REFERENCE_KEYS.has(key) && typeof entry === "string"
				? (ids.get(entry) ?? entry)
				: key === "explicitList" && Array.isArray(entry)
					? entry.map((id) => ids.get(id) ?? id)
					: remapReferences(entry, ids),
		]),
	);
}

export function pasteClipboard({
	components,
	widgetRefs,
	clipboard,
	selectionIds,
	sourceId,
	parentId,
	duplicate = false,
}: {
	components: Map<string, SurfaceComponent>;
	widgetRefs: Map<string, IWidgetRef>;
	clipboard: BuilderClipboard;
	selectionIds: string[];
	sourceId: string;
	parentId?: string;
	duplicate?: boolean;
}): {
	components: Map<string, SurfaceComponent>;
	widgetRefs: Map<string, IWidgetRef>;
	rootIds: string[];
	consumedCut: boolean;
} | null {
	const targetId = resolvePasteParent(components, selectionIds, parentId);
	if (!targetId) return null;
	const rootIds = clipboard.rootIds;
	if (!rootIds.length) return null;

	// Cuts from another editor or a previous session behave as copies.
	if (clipboard.cut && clipboard.sourceId === sourceId && !duplicate) {
		if (!rootIds.every((id) => canMoveComponent(components, id, targetId)))
			return null;
		let next = components;
		for (const id of rootIds) next = moveComponentInTree(next, id, targetId);
		return { components: next, widgetRefs, rootIds, consumedCut: true };
	}

	const targets = new Map<string, string>();
	for (const id of rootIds) {
		const target = duplicate ? findComponentParent(components, id) : targetId;
		if (!target || !canAcceptComponentChildren(components.get(target)))
			return null;
		// Named content slots cannot receive an extra sibling reference.
		if (duplicate && !getExplicitChildren(components.get(target)).includes(id))
			return null;
		targets.set(id, target);
	}

	const ids = new Map<string, string>();
	for (const component of clipboard.components) {
		ids.set(component.id, `${component.component.type}-${createId()}`);
	}
	if (rootIds.some((id) => !ids.has(id))) return null;
	const next = new Map(components);
	const nextRefs = new Map(widgetRefs);
	for (const component of clipboard.components) {
		const cloned = structuredClone(component);
		cloned.id = ids.get(component.id) ?? component.id;
		cloned.component = remapReferences(cloned.component, ids) as A2UIComponent;
		if ("id" in cloned.component) cloned.component.id = cloned.id;
		if (
			cloned.component.type === "widgetInstance" ||
			cloned.component.type === "microWidgetInstance"
		) {
			const originalInstance = cloned.component.instanceId;
			const instanceId = `widget-${createId()}`;
			cloned.component.instanceId = instanceId;
			const ref =
				clipboard.widgetRefs?.[originalInstance] ??
				widgetRefs.get(originalInstance);
			if (ref) nextRefs.set(instanceId, structuredClone(ref));
		}
		next.set(cloned.id, cloned);
	}

	for (const id of rootIds) {
		const target = targets.get(id);
		const newId = ids.get(id);
		if (!target || !newId) return null;
		const parent = next.get(target);
		if (!parent) return null;
		const children = [...getExplicitChildren(parent)];
		const insertionIndex = duplicate
			? children.indexOf(id) + 1
			: children.length;
		children.splice(insertionIndex, 0, newId);
		next.set(target, {
			...parent,
			component: {
				...parent.component,
				children: { explicitList: children },
			} as A2UIComponent,
		});
	}
	return {
		components: next,
		widgetRefs: nextRefs,
		rootIds: rootIds.map((id) => ids.get(id) ?? id),
		consumedCut: false,
	};
}
