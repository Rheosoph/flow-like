import type { IHomeDefaults, IHomeLayout, IHomeWidget } from "./types";

export const MAX_HOME_WIDGETS = 80;
export const HOME_ROW_HEIGHT = 88;
export const HOME_GRID_GAP = 16;

function record(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function normalizeHomeLayout(value: unknown): IHomeLayout | null {
	if (
		!record(value) ||
		value.version !== 1 ||
		!Array.isArray(value.widgets) ||
		value.widgets.length > MAX_HOME_WIDGETS
	)
		return null;
	const ids = new Set<string>();
	const widgets: IHomeWidget[] = [];
	for (const item of value.widgets) {
		if (
			!record(item) ||
			typeof item.id !== "string" ||
			!item.id ||
			ids.has(item.id) ||
			typeof item.type !== "string" ||
			!item.type
		)
			return null;
		ids.add(item.id);
		const size = record(item.size) ? item.size : {};
		const appearance = record(item.appearance) ? item.appearance : {};
		widgets.push({
			id: item.id,
			type: item.type,
			title: typeof item.title === "string" ? item.title : undefined,
			description:
				typeof item.description === "string" ? item.description : undefined,
			size: {
				columns: clampSize(size.columns, 6),
				rows: clampSize(size.rows, 3),
			},
			appearance: {
				variant:
					typeof appearance.variant === "string" ? appearance.variant : "card",
				accent:
					typeof appearance.accent === "string" ? appearance.accent : "neutral",
			},
			config: record(item.config) ? item.config : {},
		});
	}
	return {
		version: 1,
		title: typeof value.title === "string" ? value.title : undefined,
		description:
			typeof value.description === "string" ? value.description : undefined,
		widgets,
	};
}

function clampSize(value: unknown, fallback: number) {
	return typeof value === "number" && Number.isFinite(value)
		? Math.min(12, Math.max(1, Math.round(value)))
		: fallback;
}

export function resolveHomeLayout(
	custom: unknown,
	defaults: IHomeDefaults | undefined,
	fallback: IHomeLayout,
): {
	layout: IHomeLayout;
	source: "personal" | "profile" | "main" | "bundled";
} {
	const personal = normalizeHomeLayout(custom);
	if (personal) return { layout: personal, source: "personal" };
	const profile = normalizeHomeLayout(defaults?.profile?.layout);
	if (profile) return { layout: profile, source: "profile" };
	const main = normalizeHomeLayout(defaults?.main?.layout);
	return main
		? { layout: main, source: "main" }
		: { layout: fallback, source: "bundled" };
}

export function responsiveHomeColumns(width: number) {
	return width < 600 ? 1 : width < 1050 ? 6 : 12;
}

export function homeWidgetSpan(columns: number, gridColumns: number) {
	return gridColumns === 1
		? 1
		: Math.min(
				gridColumns,
				Math.max(1, Math.ceil((columns * gridColumns) / 12)),
			);
}

export function moveHomeWidget(
	layout: IHomeLayout,
	id: string,
	targetId: string,
): IHomeLayout {
	const from = layout.widgets.findIndex((widget) => widget.id === id);
	const to = layout.widgets.findIndex((widget) => widget.id === targetId);
	if (from < 0 || to < 0 || from === to) return layout;
	const widgets = [...layout.widgets];
	widgets.splice(to, 0, ...widgets.splice(from, 1));
	return { ...layout, widgets };
}

export function minimumHomeWidgetRows(widget: IHomeWidget): number {
	if (widget.type === "data")
		return ["stat", "metricstrip", "progress", "bullet"].includes(
			String(widget.config.visualization),
		)
			? 2
			: 3;
	if (widget.type === "app-embed") return 3;
	return ["greeting", "flowpilot", "quick-actions"].includes(widget.type)
		? 1
		: 2;
}
