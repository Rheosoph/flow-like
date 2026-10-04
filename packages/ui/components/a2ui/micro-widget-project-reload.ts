import type { AppPackageWidget } from "../../lib/package-widgets";
import type { IPage } from "../../state/backend-state/page-state";
import {
	type MicroWidgetReloadReport,
	findMicroWidgetUpdate,
	reloadMicroWidgetInstance,
} from "./micro-widget-reload";
import type { MicroWidgetInstanceComponent, SurfaceComponent } from "./types";

interface ReloadSummary {
	count: number;
	report: MicroWidgetReloadReport;
	unavailable: string[];
}

export function isUnavailableMicroWidget(
	installed: readonly AppPackageWidget[] | undefined,
	placed: Pick<MicroWidgetInstanceComponent, "packageId" | "widgetId">,
): boolean {
	return Boolean(
		installed?.some((entry) => entry.packageId === placed.packageId) &&
			!installed.some(
				(entry) =>
					entry.packageId === placed.packageId &&
					entry.widget.id === placed.widgetId,
			),
	);
}

export function unavailableMicroWidgetsError(
	references: readonly string[],
): Error {
	return new Error(
		`These widgets are unavailable in the installed packages: ${references.join(", ")}.`,
	);
}

function createSummary(): ReloadSummary {
	return {
		count: 0,
		unavailable: [],
		report: {
			converted: [],
			defaulted: [],
			reset: [],
			removed: [],
			undeclaredEvents: [],
		},
	};
}

function reloadComponent(
	surface: SurfaceComponent,
	installed: readonly AppPackageWidget[] | undefined,
	summary: ReloadSummary,
): SurfaceComponent {
	if (surface.component.type !== "microWidgetInstance") return surface;
	if (isUnavailableMicroWidget(installed, surface.component)) {
		const reference = `${surface.component.packageId}/${surface.component.widgetId}`;
		if (!summary.unavailable.includes(reference)) {
			summary.unavailable.push(reference);
		}
		return surface;
	}
	const update = findMicroWidgetUpdate(installed, surface.component);
	if (!update) return surface;
	const { component, report } = reloadMicroWidgetInstance(
		surface.component,
		update,
	);
	summary.count += 1;
	for (const key of Object.keys(report) as (keyof MicroWidgetReloadReport)[]) {
		for (const name of report[key]) {
			if (!summary.report[key].includes(name)) summary.report[key].push(name);
		}
	}
	return { ...surface, component };
}

function reloadComponents(
	components: SurfaceComponent[],
	installed: readonly AppPackageWidget[] | undefined,
	summary: ReloadSummary,
): SurfaceComponent[] {
	const countBefore = summary.count;
	const reloaded = components.map((component) =>
		reloadComponent(component, installed, summary),
	);
	return summary.count === countBefore ? components : reloaded;
}

export function reloadMicroWidgetsInComponents(
	components: SurfaceComponent[],
	installed: readonly AppPackageWidget[] | undefined,
): ReloadSummary & { components: SurfaceComponent[] } {
	const summary = createSummary();
	return {
		components: reloadComponents(components, installed, summary),
		...summary,
	};
}

/** Updates package widgets in every component store carried by a page. */
export function reloadMicroWidgetsInPage<
	T extends Pick<IPage, "components" | "content" | "widgetRefs">,
>(
	page: T,
	installed: readonly AppPackageWidget[] | undefined,
): ReloadSummary & { page: T } {
	const summary = createSummary();
	const components = reloadComponents(
		page.components ?? [],
		installed,
		summary,
	);
	let content = page.content;
	const contentBefore = summary.count;
	const reloadedContent = (page.content ?? []).map((entry) => {
		if (!("Component" in entry)) return entry;
		const component = reloadComponent(entry.Component, installed, summary);
		return component === entry.Component
			? entry
			: { ...entry, Component: component };
	});
	if (summary.count !== contentBefore) content = reloadedContent;

	let widgetRefs = page.widgetRefs;
	for (const [id, widget] of Object.entries(page.widgetRefs ?? {})) {
		const reloaded = reloadComponents(
			widget.components ?? [],
			installed,
			summary,
		);
		if (reloaded === widget.components) continue;
		if (!widgetRefs || widgetRefs === page.widgetRefs) {
			widgetRefs = { ...page.widgetRefs };
		}
		widgetRefs[id] = { ...widget, components: reloaded };
	}

	return {
		page:
			summary.count === 0
				? page
				: {
						...page,
						components,
						content,
						...(widgetRefs ? { widgetRefs } : {}),
					},
		...summary,
	};
}
