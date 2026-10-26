import type { SurfaceComponent, WidgetAction } from "../components/a2ui/types";

type ActionValue = {
	name: string;
	context?: Record<string, unknown>;
};

type ComponentActionData = {
	actions?: ActionValue[];
	eventHandlers?: Record<string, ActionValue[]>;
};

const ACTION_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]*$/;

export type WidgetActionIdIssue = "empty" | "invalid" | "duplicate";

export function normalizeWidgetActionId(raw: string): string {
	return raw.trim().replace(/\s+/g, "-");
}

export function checkWidgetActionId(
	id: string,
	takenIds: readonly string[],
): WidgetActionIdIssue | null {
	if (id.length === 0) return "empty";
	if (!ACTION_ID_PATTERN.test(id)) return "invalid";
	if (takenIds.includes(id)) return "duplicate";
	return null;
}

export interface UndeclaredWidgetAction {
	actionId: string;
	componentIds: string[];
}

function widgetEventId(action: ActionValue) {
	const id = action.name === "widget_event" ? action.context?.actionId : "";
	return typeof id === "string" ? id : "";
}

function componentWidgetEventIds(component: SurfaceComponent) {
	const data = component.component as ComponentActionData | undefined;
	return [
		...(data?.actions ?? []),
		...Object.values(data?.eventHandlers ?? {}).flat(),
	]
		.map(widgetEventId)
		.filter(Boolean);
}

/**
 * Every `widget_event` id the widget's own components fire that `declared` lacks. Pages only offer
 * declared ids for binding, so an undeclared one can never reach a flow.
 */
export function findUndeclaredWidgetActions(
	components: readonly SurfaceComponent[],
	declared: readonly Pick<WidgetAction, "id">[] | undefined,
): UndeclaredWidgetAction[] {
	const declaredIds = new Set((declared ?? []).map((action) => action.id));
	const usage = new Map<string, Set<string>>();
	for (const component of components) {
		for (const id of componentWidgetEventIds(component)) {
			if (declaredIds.has(id)) continue;
			usage.set(id, (usage.get(id) ?? new Set()).add(component.id));
		}
	}
	return [...usage].map(([actionId, users]) => ({
		actionId,
		componentIds: [...users],
	}));
}

/** The widget event `action` fires and its declaration; `undeclared` holds the id when it has none. */
export function resolveWidgetEventTarget<T extends Pick<WidgetAction, "id">>(
	action: ActionValue,
	declared: readonly T[] | undefined,
) {
	const actionId = widgetEventId(action);
	const declaration = declared?.find((entry) => entry.id === actionId);
	const undeclared =
		declared && actionId && !declaration ? actionId : undefined;
	return { actionId, declaration, undeclared };
}

/** Appends a declaration for each id `actions` does not already declare. */
export function declareWidgetActions(
	actions: readonly WidgetAction[] | undefined,
	ids: readonly string[],
): WidgetAction[] {
	const next = [...(actions ?? [])];
	const taken = new Set(next.map((action) => action.id));
	for (const id of ids) {
		if (taken.has(id)) continue;
		taken.add(id);
		next.push({ id, label: id, contextSchema: [] });
	}
	return next;
}

function renameInActions(
	actions: ActionValue[],
	oldId: string,
	newId: string,
): ActionValue[] | null {
	let changed = false;
	const next = actions.map((action) => {
		if (action.name !== "widget_event" || action.context?.actionId !== oldId) {
			return action;
		}
		changed = true;
		return { ...action, context: { ...action.context, actionId: newId } };
	});
	return changed ? next : null;
}

/**
 * Rewrites every `widget_event` reference to `oldId` inside the widget's own components.
 * Returns the original array when nothing referenced the old id.
 */
export function renameWidgetActionInComponents(
	components: SurfaceComponent[],
	oldId: string,
	newId: string,
): SurfaceComponent[] {
	let touched = false;
	const next = components.map((component) => {
		const data = component.component as ComponentActionData | undefined;
		if (!data) return component;

		const legacy = data.actions
			? renameInActions(data.actions, oldId, newId)
			: null;

		let handlers: Record<string, ActionValue[]> | null = null;
		for (const [event, actions] of Object.entries(data.eventHandlers ?? {})) {
			const renamed = renameInActions(actions, oldId, newId);
			if (!renamed) continue;
			handlers ??= { ...data.eventHandlers };
			handlers[event] = renamed;
		}

		if (!legacy && !handlers) return component;
		touched = true;
		return {
			...component,
			component: {
				...data,
				...(legacy ? { actions: legacy } : {}),
				...(handlers ? { eventHandlers: handlers } : {}),
			},
		} as SurfaceComponent;
	});
	return touched ? next : components;
}
