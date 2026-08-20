import type { SurfaceComponent } from "../components/a2ui/types";

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
