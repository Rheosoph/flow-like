import { type HomeDataConfig, homeDataMeasureTitle } from "./home-data-query";

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
						? `${homeDataText(row.__series)} · ${label}`
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
