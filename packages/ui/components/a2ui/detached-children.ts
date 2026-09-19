import { getComponentChildren } from "./children";
import type { Surface, SurfaceComponent } from "./types";

type Records = NonNullable<Surface["detachedChildren"]>;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Surface component ids this component keeps rendered. A widget host also holds the
 * surface ids pushed into its internal containers (inline, or still queued for an
 * external definition); its own internal child ids are not surface components.
 */
function componentRefs(component: SurfaceComponent): string[] {
	const refs = getComponentChildren(component);
	const data = component.component as unknown as Record<string, unknown>;
	if (data?.type !== "widgetInstance") return refs;

	const definition = data.inlineWidgetDef as
		| { components?: SurfaceComponent[] }
		| undefined;
	const internal = definition?.components ?? [];
	const internalIds = new Set(internal.map((child) => child?.id));
	for (const child of internal) {
		for (const id of getComponentChildren(child)) {
			if (!internalIds.has(id)) refs.push(id);
		}
	}

	if (isRecord(data.runtimeChildUpdates)) {
		for (const updates of Object.values(data.runtimeChildUpdates)) {
			if (!Array.isArray(updates)) continue;
			for (const update of updates) {
				if (isRecord(update) && typeof update.childId === "string") {
					refs.push(update.childId);
				}
			}
		}
	}
	return refs;
}

function referenceCounts(
	components: Record<string, SurfaceComponent>,
): Map<string, number> {
	const counts = new Map<string, number>();
	for (const component of Object.values(components)) {
		for (const id of componentRefs(component)) {
			counts.set(id, (counts.get(id) ?? 0) + 1);
		}
	}
	return counts;
}

function withRecords(surface: Surface, records: Records): Surface {
	const { detachedChildren: _previous, ...rest } = surface;
	return Object.keys(records).length
		? { ...rest, detachedChildren: records }
		: rest;
}

/**
 * Remember the components an update on `elementId` left unreferenced. They stay on the
 * surface, so the run may still push them somewhere, until `pruneDetached` names `elementId`.
 */
export function recordDetached(
	before: Surface,
	after: Surface,
	elementId: string,
): Surface {
	if (after === before) return after;
	const remaining = referenceCounts(after.components);
	const detached = [...referenceCounts(before.components).keys()].filter(
		(id) => !remaining.has(id) && after.components[id] !== undefined,
	);
	if (!detached.length) return after;

	const existing = after.detachedChildren?.[elementId] ?? [];
	return withRecords(after, {
		...after.detachedChildren,
		[elementId]: [...new Set([...existing, ...detached])],
	});
}

/** A re-created component is a new one: pruning must not delete it for its predecessor's detach. */
export function forgetDetached(
	surface: Surface,
	componentIds: readonly string[],
): Surface {
	const records = surface.detachedChildren;
	if (!records) return surface;
	const forget = new Set(componentIds);
	let changed = false;
	const next: Records = {};
	for (const [elementId, detached] of Object.entries(records)) {
		const kept = detached.filter((id) => !forget.has(id));
		if (kept.length !== detached.length) changed = true;
		if (kept.length) next[elementId] = kept;
	}
	return changed ? withRecords(surface, next) : surface;
}

/**
 * Delete what the run detached from `elementIds` and nothing references anymore, with
 * every descendant that deletion leaves unreferenced. The surface root is never deleted.
 */
export function pruneDetached(
	surface: Surface,
	elementIds: readonly string[],
): Surface {
	const records = surface.detachedChildren;
	if (!records) return surface;

	const remainingRecords: Records = { ...records };
	const pending: string[] = [];
	for (const elementId of elementIds) {
		const detached = remainingRecords[elementId];
		if (!detached) continue;
		pending.push(...detached);
		delete remainingRecords[elementId];
	}
	if (!pending.length) return surface;

	const components = { ...surface.components };
	const counts = referenceCounts(components);
	while (pending.length) {
		const id = pending.pop() as string;
		const component = components[id];
		if (
			!component ||
			id === surface.rootComponentId ||
			(counts.get(id) ?? 0) > 0
		) {
			continue;
		}
		delete components[id];
		for (const childId of componentRefs(component)) {
			const count = (counts.get(childId) ?? 0) - 1;
			if (count > 0) {
				counts.set(childId, count);
			} else {
				counts.delete(childId);
				pending.push(childId);
			}
		}
	}

	return withRecords({ ...surface, components }, remainingRecords);
}
