import { type HomeDataConfig, homeDataMeasureTitle } from "./home-data-query";

export const HOME_DATA_COLORS = [
	"var(--chart-1)",
	"#60a5fa",
	"#a78bfa",
	"#2dd4bf",
	"#f472b6",
	"#facc15",
	"#fb923c",
	"#818cf8",
];

export function homeDataPresentationRequirement(
	config: HomeDataConfig,
): string | null {
	const view = config.visualization;
	if (
		["progress", "gauge", "bullet"].includes(view) &&
		(config.target === null || config.target <= 0)
	)
		return "Set a target greater than zero in widget settings.";
	if (["scatter", "graph"].includes(view) && (!config.xField || !config.yField))
		return view === "graph"
			? "Choose source and target ID columns in widget settings."
			: "Choose numeric X and Y fields in widget settings.";
	if (
		["heatmap", "pivot", "sankey"].includes(view) &&
		(!config.groupBy || !config.seriesBy)
	)
		return "Choose a group and a series column in widget settings.";
	if (
		view === "calendar" &&
		(!config.groupBy || config.timeBucket !== "day" || config.seriesBy)
	)
		return "Choose a date group, daily grouping, and no series in widget settings.";
	if (view === "kanban" && !config.groupBy)
		return "Choose a status column in widget settings.";
	if (["timeline", "recordcalendar"].includes(view) && !config.xField)
		return "Choose a date column in widget settings.";
	if (view === "boxplot" && !config.yField)
		return "Choose a numeric distribution field in widget settings.";
	if (view === "histogram" && !config.groupBy)
		return "Choose a numeric field and bin width in widget settings.";
	return null;
}

export function homeDataShortLabel(value: unknown, length = 18): string {
	const label = homeDataText(value);
	return label.length > length ? `${label.slice(0, length - 1)}…` : label;
}

export function homeDataAxisValue(
	value: unknown,
	config: HomeDataConfig,
): string {
	const number = homeDataNumber(value);
	if (number === null) return "";
	return new Intl.NumberFormat(undefined, {
		notation: Math.abs(number) >= 10_000 ? "compact" : "standard",
		maximumFractionDigits: Math.abs(number) < 1 ? 2 : 1,
		...(config.format === "percent"
			? { style: "percent" }
			: config.format === "currency"
				? {
						style: "currency",
						currency: config.currency,
						currencyDisplay: "narrowSymbol",
					}
				: {}),
	}).format(number);
}

export function homeDataCategoryLabel(value: unknown, length = 16): string {
	const text = homeDataText(value);
	if (/^\d{4}-\d{2}-\d{2}(T| |$)/.test(text)) {
		const date = new Date(text.slice(0, 10));
		if (!Number.isNaN(date.getTime()))
			return date.toLocaleDateString(undefined, {
				month: "short",
				day: "numeric",
				timeZone: "UTC",
			});
	}
	return homeDataShortLabel(value, length);
}

export function homeDataText(value: unknown): string {
	if (value === null || value === undefined) return "No value";
	return typeof value === "object" ? JSON.stringify(value) : String(value);
}
export function homeDataNumber(value: unknown): number | null {
	if (
		value === null ||
		value === undefined ||
		(typeof value !== "number" && typeof value !== "string") ||
		(typeof value === "string" && !value.trim())
	)
		return null;
	const number = typeof value === "number" ? value : Number(value);
	return Number.isFinite(number) ? number : null;
}

/** Pivot server-aggregated rows without recomputing totals over a limited result. */
export function homeDataChartSeries(
	rows: Record<string, unknown>[],
	config: HomeDataConfig,
) {
	const series: { key: string; label: string }[] = [];
	const seriesById = new Map<string, string>();
	const points = new Map<string, Record<string, unknown>>();
	for (const row of rows) {
		const category = config.groupBy ? homeDataText(row.__group) : "Total";
		const groupId = JSON.stringify(row.__group ?? null);
		const point = points.get(groupId) ?? { name: category };
		points.set(groupId, point);
		(config.visualization === "percentstacked"
			? config.measures.slice(0, 1)
			: config.measures
		).forEach((measure, index) => {
			const id = JSON.stringify([config.seriesBy ? row.__series : null, index]);
			let key = seriesById.get(id);
			if (!key) {
				key = `value_${series.length}`;
				seriesById.set(id, key);
				const label = homeDataMeasureTitle(measure);
				series.push({
					key,
					label: config.seriesBy
						? config.measures.length === 1
							? homeDataText(row.__series)
							: `${homeDataText(row.__series)} · ${label}`
						: label,
				});
			}
			point[key] = homeDataNumber(
				row[
					config.visualization === "percentstacked"
						? "__share"
						: `__measure_${index}`
				],
			);
		});
	}
	return { points: [...points.values()], series };
}

export function keyHomeDataRows(rows: Record<string, unknown>[]) {
	const occurrences = new Map<string, number>();
	return rows.map((row) => {
		const content = JSON.stringify(row);
		const occurrence = occurrences.get(content) ?? 0;
		occurrences.set(content, occurrence + 1);
		return { key: `${content}:${occurrence}`, row };
	});
}

export function homeDataNetwork(
	rows: Record<string, unknown>[],
	source: string,
	target: string,
) {
	const ids = new Set<string>();
	const pairs = new Set<string>();
	const links: { source: string; target: string }[] = [];
	for (const row of rows) {
		if (
			row[source] === null ||
			row[source] === undefined ||
			row[target] === null ||
			row[target] === undefined
		)
			continue;
		const from = homeDataText(row[source]);
		const to = homeDataText(row[target]);
		const key = JSON.stringify([from, to]);
		if (pairs.has(key)) continue;
		pairs.add(key);
		ids.add(from);
		ids.add(to);
		links.push({ source: from, target: to });
	}
	return { nodes: [...ids].map((id) => ({ id })), links };
}
