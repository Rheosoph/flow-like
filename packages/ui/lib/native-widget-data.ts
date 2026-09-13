import {
	type HomeDataSourceContext,
	buildHomeDataQuery,
	formatHomeDataValue,
	homeDataColumns,
	homeDataMeasureTitle,
	homeOntologyColumns,
} from "../components/home/home-data-query";
import type { IBackendState } from "../state/backend-state";
import type { ExecuteSqlResult } from "../state/backend-state/query-state";
import {
	type NativeChartType,
	type NativeCustomWidget,
	type NativeWidgetChart,
	type NativeWidgetChartDefinition,
	nativeWidgetShell,
	truncateNativeWidgetText,
} from "./native-widget";

export function assertNativeWidgetActive(signal?: AbortSignal) {
	if (signal?.aborted)
		throw new DOMException("Widget refresh cancelled", "AbortError");
}

export async function assertNativeWidgetApp(
	backend: IBackendState,
	appId: string,
	viewerId?: string,
	signal?: AbortSignal,
) {
	assertNativeWidgetActive(signal);
	const profile = await backend.userState.getProfile();
	assertNativeWidgetActive(signal);
	if (!profile.apps?.some((app) => app.app_id === appId))
		throw new Error("This app is no longer in the selected profile.");
	if (!viewerId && !(await backend.isLocalOnly?.(appId)))
		throw new Error("Sign in to refresh this widget.");
	assertNativeWidgetActive(signal);
}

const label = (value: unknown) =>
	value === null || value === undefined
		? "No value"
		: typeof value === "object"
			? "Value"
			: truncateNativeWidgetText(String(value), 100);
const number = (value: unknown) =>
	typeof value === "number"
		? value
		: typeof value === "string" && value.trim()
			? Number(value)
			: Number.NaN;

export function nativeChartFromResult(
	definition: NativeWidgetChartDefinition,
	result: ExecuteSqlResult,
): { chart: NativeWidgetChart; warnings: string[] } {
	const config = definition.data;
	const warnings: string[] = [];
	const series = new Set<string>();
	const chart: NativeWidgetChart = {
		type: config.visualization as NativeChartType,
		points: [],
		format: {
			style: config.format,
			currency: config.currency,
			decimals: config.decimals,
		},
		...(config.target !== null ? { target: config.target } : {}),
	};
	let omitted = false;
	let invalid = false;
	for (const [rowIndex, row] of result.rows.entries()) {
		for (const [measureIndex, measure] of config.measures.entries()) {
			const value = number(row[`__measure_${measureIndex}`]);
			if (!Number.isFinite(value)) {
				invalid = true;
				continue;
			}
			if (["pie", "donut"].includes(chart.type) && value < 0) {
				invalid = true;
				continue;
			}
			const measureName = homeDataMeasureTitle(measure);
			const seriesName = config.seriesBy
				? `${label(row.__series)}${config.measures.length > 1 ? ` · ${measureName}` : ""}`
				: measureName;
			if (
				chart.points.length >= 120 ||
				(!series.has(seriesName) && series.size >= 6)
			) {
				omitted = true;
				continue;
			}
			series.add(seriesName);
			chart.points.push({
				id: `${rowIndex}:${measureIndex}`,
				label: config.groupBy
					? label(row.__group)
					: truncateNativeWidgetText(measureName, 100),
				series: truncateNativeWidgetText(seriesName, 100),
				value,
				formattedValue: truncateNativeWidgetText(
					formatHomeDataValue(value, config),
					50,
				),
			});
		}
	}
	if (["stat", "progress", "gauge"].includes(chart.type)) {
		chart.points = chart.points.slice(0, 1);
		chart.value = chart.points[0]?.value;
		chart.formattedValue = chart.points[0]?.formattedValue;
	}
	if (result.truncated || omitted)
		warnings.push(
			"Showing a limited set of groups and series. Narrow the filters for a complete breakdown.",
		);
	if (invalid)
		warnings.push(
			["pie", "donut"].includes(chart.type)
				? "Values that cannot form a slice were omitted."
				: "Empty or non-numeric values were omitted.",
		);
	return { chart, warnings };
}

export async function loadNativeWidgetChart(
	backend: IBackendState,
	definition: NativeWidgetChartDefinition,
	options: { viewerId?: string; signal?: AbortSignal; now?: Date } = {},
): Promise<NativeCustomWidget> {
	const { viewerId, signal } = options;
	await assertNativeWidgetApp(backend, definition.appId, viewerId, signal);
	const config = definition.data;
	if (
		["progress", "gauge"].includes(config.visualization) &&
		(config.target === null || config.target <= 0)
	)
		throw new Error("Set a positive target for this widget.");
	const personal = config.scope === "personal";
	const context: HomeDataSourceContext = { viewerId, now: options.now };
	if (config.sourceKind === "ontology") {
		context.overlay = await backend.graphState.getOverlay(
			config.appId,
			config.ontologyId,
			personal,
		);
		context.columns = homeOntologyColumns(context.overlay, config.objectType);
	} else if (config.sourceKind === "query") {
		context.savedQuery = await backend.queryState.getSavedQuery(
			config.appId,
			config.queryId,
			personal,
		);
	} else {
		context.columns = homeDataColumns(
			await backend.dbState.getSchema(config.appId, config.table, personal),
		);
	}
	assertNativeWidgetActive(signal);
	const query = buildHomeDataQuery(config, context);
	const result = await backend.queryState.executeSql(
		config.appId,
		query,
		personal,
	);
	assertNativeWidgetActive(signal);
	const { chart, warnings } = nativeChartFromResult(definition, result);
	return {
		...nativeWidgetShell(definition, options.now),
		state: chart.points.length ? "ready" : "empty",
		chart,
		warnings,
		...(chart.points.length
			? {}
			: { message: "No values match these settings." }),
	};
}
