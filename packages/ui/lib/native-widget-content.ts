import { parseAppRouteTarget } from "./app-route-url";
import {
	NATIVE_CHART_TYPES,
	type NativeCustomWidget,
	type NativeWidgetDefinition,
} from "./native-widget";
import { notificationIconSource } from "./notification-icon";

type Value = Record<string, unknown>;
const object = (value: unknown): value is Value =>
	!!value && typeof value === "object" && !Array.isArray(value);
const keys = (value: Value, allowed: string) =>
	Object.keys(value).every((key) => allowed.split(" ").includes(key));
const text = (value: unknown, max: number) =>
	typeof value === "string" && new TextEncoder().encode(value).length <= max;
const optionalText = (value: unknown, max: number) =>
	value === undefined || text(value, max);
const numeric = (value: unknown) =>
	typeof value === "number" && Number.isFinite(value);
const optionalNumber = (value: unknown) =>
	value === undefined || numeric(value);
const choice = (value: unknown, options: string) =>
	value === undefined || options.split(" ").includes(String(value));

function action(value: unknown, appId: string): boolean {
	if (
		!object(value) ||
		!keys(value, "kind appId path queryParams") ||
		value.kind !== "open_app" ||
		value.appId !== appId
	)
		return false;
	try {
		parseAppRouteTarget(value.path, value.queryParams);
		return true;
	} catch {
		return false;
	}
}
function chart(value: unknown): boolean {
	if (
		!object(value) ||
		!keys(value, "type points format target value formattedValue") ||
		!NATIVE_CHART_TYPES.some(([type]) => type === value.type) ||
		!Array.isArray(value.points) ||
		value.points.length > 120 ||
		!object(value.format)
	)
		return false;
	const format = value.format;
	if (
		!keys(format, "style currency decimals") ||
		!["number", "currency", "percent"].includes(String(format.style)) ||
		!text(format.currency, 8) ||
		!Number.isInteger(format.decimals) ||
		Number(format.decimals) < 0 ||
		Number(format.decimals) > 6 ||
		!optionalNumber(value.target) ||
		!optionalNumber(value.value) ||
		!optionalText(value.formattedValue, 160)
	)
		return false;
	const ids = new Set();
	const series = new Set();
	return value.points.every((point) => {
		if (
			!object(point) ||
			!keys(point, "id label series value formattedValue") ||
			!text(point.id, 256) ||
			!point.id ||
			ids.has(point.id) ||
			!text(point.label, 512) ||
			!text(point.series, 512) ||
			!text(point.formattedValue, 160) ||
			!numeric(point.value)
		)
			return false;
		ids.add(point.id);
		series.add(point.series);
		return (
			series.size <= 6 &&
			(!["pie", "donut"].includes(String(value.type)) ||
				Number(point.value) >= 0)
		);
	});
}
function page(
	value: unknown,
	appId: string,
	depth: number,
	visited: Set<string>,
	media: { count: number },
): boolean {
	if (
		depth > 8 ||
		visited.size >= 60 ||
		!object(value) ||
		!keys(
			value,
			"id kind text role tone alignment spacing columns children image progress value table chart action",
		) ||
		!text(value.id, 256) ||
		!value.id ||
		visited.has(String(value.id)) ||
		![
			"column",
			"row",
			"stack",
			"grid",
			"card",
			"text",
			"image",
			"icon",
			"badge",
			"progress",
			"divider",
			"spacer",
			"table",
			"chart",
			"link",
		].includes(String(value.kind))
	)
		return false;
	visited.add(String(value.id));
	if (
		!optionalText(value.text, 4096) ||
		!optionalText(value.value, 256) ||
		!choice(value.role, "title headline body caption") ||
		!choice(value.tone, "default muted accent success warning danger") ||
		!choice(value.alignment, "leading center trailing") ||
		!optionalNumber(value.spacing) ||
		(value.spacing !== undefined &&
			(Number(value.spacing) < 0 || Number(value.spacing) > 32)) ||
		(value.columns !== undefined &&
			(!Number.isInteger(value.columns) ||
				Number(value.columns) < 1 ||
				Number(value.columns) > 6)) ||
		(value.progress !== undefined &&
			(!numeric(value.progress) ||
				Number(value.progress) < 0 ||
				Number(value.progress) > 1)) ||
		(value.action !== undefined && !action(value.action, appId))
	)
		return false;
	if (value.image !== undefined) {
		if (
			++media.count > 4 ||
			!object(value.image) ||
			!keys(value.image, "png text template") ||
			(value.image.template !== undefined &&
				typeof value.image.template !== "boolean")
		)
			return false;
		if (value.image.png !== undefined) {
			if (!text(value.image.png, 43_692)) return false;
			try {
				const data = atob(String(value.image.png));
				if (data.length > 32_768 || !data.startsWith("\x89PNG\r\n\x1a\n"))
					return false;
			} catch {
				return false;
			}
		} else if (notificationIconSource(value.image.text)?.kind !== "emoji")
			return false;
	}
	if (value.kind === "chart" && !chart(value.chart)) return false;
	if (value.kind === "table") {
		const table = value.table;
		if (
			!object(table) ||
			!keys(table, "columns rows") ||
			!Array.isArray(table.columns) ||
			!table.columns.length ||
			table.columns.length > 6 ||
			!table.columns.every((column) => text(column, 160)) ||
			!Array.isArray(table.rows) ||
			table.rows.length > 8
		)
			return false;
		const columns = table.columns.length;
		if (
			!table.rows.every(
				(row) =>
					Array.isArray(row) &&
					row.length === columns &&
					row.every((cell) => text(cell, 256)),
			)
		)
			return false;
	}
	return (
		value.children === undefined ||
		(Array.isArray(value.children) &&
			value.children.every((child) =>
				page(child, appId, depth + 1, visited, media),
			))
	);
}

/** Validate persisted display data before rendering or forwarding it to WidgetKit. */
export function isNativeWidgetContent(
	value: unknown,
	definition: NativeWidgetDefinition,
	now = Date.now(),
): value is NativeCustomWidget {
	if (
		!object(value) ||
		!keys(
			value,
			"id title kind appId updatedAt staleAt expiresAt state message warnings accent action chart page",
		) ||
		value.id !== definition.id ||
		value.kind !== definition.kind ||
		value.appId !== definition.appId ||
		!text(value.title, 512) ||
		!value.title ||
		!action(value.action, definition.appId)
	)
		return false;
	const updated = Date.parse(String(value.updatedAt));
	const stale = Date.parse(String(value.staleAt));
	const expiry = Date.parse(String(value.expiresAt));
	if (
		!Number.isFinite(updated) ||
		!Number.isFinite(stale) ||
		!Number.isFinite(expiry) ||
		updated > now + 60_000 ||
		stale < updated ||
		expiry <= now ||
		expiry < stale ||
		expiry - updated > 7 * 86_400_000 + 60_000 ||
		!["ready", "empty", "error", "unsupported", "unavailable"].includes(
			String(value.state),
		) ||
		!optionalText(value.message, 1024) ||
		!choice(value.accent, "orange blue teal purple")
	)
		return false;
	if (
		value.warnings !== undefined &&
		(!Array.isArray(value.warnings) ||
			value.warnings.length > 8 ||
			!value.warnings.every((warning) => text(warning, 256)))
	)
		return false;
	if (
		(value.page !== undefined &&
			!page(value.page, definition.appId, 1, new Set(), { count: 0 })) ||
		(value.chart !== undefined && !chart(value.chart))
	)
		return false;
	if (
		value.state === "ready" &&
		(value.kind === "chart"
			? !chart(value.chart) || value.page !== undefined
			: value.chart !== undefined ||
				!page(value.page, definition.appId, 1, new Set(), { count: 0 }))
	)
		return false;
	return new TextEncoder().encode(JSON.stringify(value)).length <= 131_072;
}
