import type { BoundValue, SurfaceComponent } from "./types";

// Wrap a raw value in the BoundValue literal that best fits it, so runtime
// `upsert_element` updates (from update_calendar / update_gantt nodes) resolve
// the same way author-time literals do.
function literalWrap(value: unknown): BoundValue {
	if (typeof value === "string") return { literalString: value };
	if (typeof value === "number") return { literalNumber: value };
	if (typeof value === "boolean") return { literalBool: value };
	if (Array.isArray(value)) return { literalOptions: value as never };
	return { literalJson: JSON.stringify(value ?? null) };
}

type Item = Record<string, unknown>;

// Read the current array behind an events/tasks prop, tolerating whichever
// BoundValue shape it currently holds (literalOptions, literalJson, or unset).
function readItems(
	componentData: Record<string, unknown>,
	key: string,
): Item[] {
	const bound = componentData[key] as
		| { literalOptions?: unknown[]; literalJson?: string }
		| undefined;
	if (bound?.literalOptions && Array.isArray(bound.literalOptions)) {
		return [...bound.literalOptions] as Item[];
	}
	if (typeof bound?.literalJson === "string") {
		try {
			const parsed = JSON.parse(bound.literalJson);
			if (Array.isArray(parsed)) return parsed as Item[];
		} catch {}
	}
	return [];
}

function withComponentData(
	component: SurfaceComponent,
	nextData: Record<string, unknown>,
): SurfaceComponent {
	return {
		...component,
		component: nextData as unknown as SurfaceComponent["component"],
	};
}

// Merge a config object (camelCase keys matching component props) onto the
// component, wrapping each provided value as a BoundValue literal.
function mergeConfig(
	componentData: Record<string, unknown>,
	config: unknown,
): Record<string, unknown> {
	if (!config || typeof config !== "object") return componentData;
	const next = { ...componentData };
	for (const [key, value] of Object.entries(
		config as Record<string, unknown>,
	)) {
		if (value === undefined) continue;
		next[key] = literalWrap(value);
	}
	return next;
}

// Applies a calendar `upsert_element` op to the component's props. See
// update_calendar.rs for the op payloads emitted by the backend node.
export function applyCalendarUpdate(
	component: SurfaceComponent,
	updateValue: Record<string, unknown>,
): SurfaceComponent {
	const componentData = component.component as unknown as Record<
		string,
		unknown
	>;
	const type = updateValue.type as string;

	switch (type) {
		case "setCalendarEvents": {
			const data = (updateValue.events ?? updateValue.data ?? []) as unknown[];
			return withComponentData(component, {
				...componentData,
				events: { literalOptions: data as never },
			});
		}
		case "addCalendarEvent": {
			const event = updateValue.event as Item;
			const events = readItems(componentData, "events");
			return withComponentData(component, {
				...componentData,
				events: { literalOptions: [...events, event] as never },
			});
		}
		case "updateCalendarEvent": {
			const patch = updateValue.event as Item;
			const id = patch?.id;
			const events = readItems(componentData, "events").map((e) =>
				e.id === id ? { ...e, ...patch } : e,
			);
			return withComponentData(component, {
				...componentData,
				events: { literalOptions: events as never },
			});
		}
		case "removeCalendarEvent": {
			const id = updateValue.id;
			const events = readItems(componentData, "events").filter(
				(e) => e.id !== id,
			);
			return withComponentData(component, {
				...componentData,
				events: { literalOptions: events as never },
			});
		}
		case "setCalendarView": {
			return withComponentData(component, {
				...componentData,
				view: { literalString: String(updateValue.view ?? "month") },
			});
		}
		case "setCalendarDate": {
			return withComponentData(component, {
				...componentData,
				date: { literalString: String(updateValue.date ?? "") },
			});
		}
		case "setCalendarConfig": {
			return withComponentData(
				component,
				mergeConfig(componentData, updateValue.config),
			);
		}
		default:
			return component;
	}
}

// Applies a gantt `upsert_element` op to the component's props. See
// update_gantt.rs for the op payloads emitted by the backend node.
export function applyGanttUpdate(
	component: SurfaceComponent,
	updateValue: Record<string, unknown>,
): SurfaceComponent {
	const componentData = component.component as unknown as Record<
		string,
		unknown
	>;
	const type = updateValue.type as string;

	const setTasks = (tasks: Item[]) =>
		withComponentData(component, {
			...componentData,
			tasks: { literalOptions: tasks as never },
		});

	switch (type) {
		case "setGanttTasks": {
			const data = (updateValue.tasks ?? updateValue.data ?? []) as unknown[];
			return setTasks(data as Item[]);
		}
		case "addGanttTask": {
			const task = updateValue.task as Item;
			return setTasks([...readItems(componentData, "tasks"), task]);
		}
		case "updateGanttTask": {
			const patch = updateValue.task as Item;
			const id = patch?.id;
			return setTasks(
				readItems(componentData, "tasks").map((t) =>
					t.id === id ? { ...t, ...patch } : t,
				),
			);
		}
		case "removeGanttTask": {
			const id = updateValue.id;
			return setTasks(
				readItems(componentData, "tasks").filter((t) => t.id !== id),
			);
		}
		case "setGanttProgress": {
			const id = updateValue.id;
			const progress = updateValue.progress as number;
			return setTasks(
				readItems(componentData, "tasks").map((t) =>
					t.id === id ? { ...t, progress } : t,
				),
			);
		}
		case "addGanttDependency": {
			const fromId = updateValue.fromId as string;
			const toId = updateValue.toId as string;
			return setTasks(
				readItems(componentData, "tasks").map((t) => {
					if (t.id !== toId) return t;
					const deps = Array.isArray(t.dependencies)
						? (t.dependencies as string[])
						: [];
					return deps.includes(fromId)
						? t
						: { ...t, dependencies: [...deps, fromId] };
				}),
			);
		}
		case "removeGanttDependency": {
			const fromId = updateValue.fromId as string;
			const toId = updateValue.toId as string;
			return setTasks(
				readItems(componentData, "tasks").map((t) => {
					if (t.id !== toId || !Array.isArray(t.dependencies)) return t;
					return {
						...t,
						dependencies: (t.dependencies as string[]).filter(
							(d) => d !== fromId,
						),
					};
				}),
			);
		}
		case "setGanttView": {
			return withComponentData(component, {
				...componentData,
				view: { literalString: String(updateValue.view ?? "week") },
			});
		}
		case "setGanttConfig": {
			return withComponentData(
				component,
				mergeConfig(componentData, updateValue.config),
			);
		}
		default:
			return component;
	}
}
