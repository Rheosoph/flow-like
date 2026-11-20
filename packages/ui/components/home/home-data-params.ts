import type {
	CrossAppSqlSource,
	SqlParameterBinding,
	SqlParameterInfo,
	SqlParameterOptions,
} from "../../state/backend-state/query-state";
import type {
	HomeDataConfig,
	HomeDataParamControl,
	HomeParamControl,
} from "./home-data-query";

export const HOME_VIEWER_ID = "$viewer.id";
export const HOME_SELECT_OPTION_LIMIT = 50;

const NUMERIC_TYPE = /^(U?Int\d+|Float\d+|Decimal\d*)/i;
const LOWER_OPERATORS: SqlParameterBinding["operator"][] = ["gte", "gt"];
const UPPER_OPERATORS: SqlParameterBinding["operator"][] = ["lte", "lt"];

export type HomeParamValueKind = "date" | "number";

export type HomeParamEntry =
	| { type: "single"; info: SqlParameterInfo }
	| {
			type: "range";
			kind: HomeParamValueKind;
			from: SqlParameterInfo;
			to: SqlParameterInfo;
	  };

export function isNumericType(dataType: string | null | undefined): boolean {
	return Boolean(dataType && NUMERIC_TYPE.test(dataType));
}

function isDateType(dataType: string | null | undefined): boolean {
	return Boolean(dataType && /^Date(32|64)/.test(dataType));
}

function isTimestampType(dataType: string | null | undefined): boolean {
	return Boolean(dataType?.startsWith("Timestamp"));
}

export function primaryBinding(
	info: SqlParameterInfo,
): SqlParameterBinding | undefined {
	return (
		info.bindings.find(
			(binding) => binding.operator === "eq" || binding.operator === "in",
		) ?? info.bindings[0]
	);
}

export function inferParamControl(
	info: SqlParameterInfo,
	options?: { truncated?: boolean },
): HomeParamControl {
	const type = info.data_type;
	const binding = primaryBinding(info);
	if (type && /^Boolean/i.test(type)) return "toggle";
	if (isDateType(type)) return "date";
	if (isTimestampType(type)) return "datetime";
	if (type?.startsWith("List") || type?.startsWith("LargeList"))
		return "multi-select";
	if (info.bindings.some((item) => item.operator === "in"))
		return "multi-select";
	if (isNumericType(type)) return "number";
	if (binding?.operator === "like" || binding?.operator === "ilike")
		return "text";
	if (binding?.operator === "eq" || binding?.operator === "neq")
		return options?.truncated ? "combobox" : "select";
	return "text";
}

export function effectiveParamControl(
	info: SqlParameterInfo,
	override: HomeDataParamControl | undefined,
	options?: { truncated?: boolean },
): HomeParamControl {
	return override?.control ?? inferParamControl(info, options);
}

function rangeKind(info: SqlParameterInfo): HomeParamValueKind | null {
	const control = inferParamControl(info);
	if (control === "date" || control === "datetime") return "date";
	if (control === "number") return "number";
	return null;
}

function boundOn(
	info: SqlParameterInfo,
	operators: SqlParameterBinding["operator"][],
) {
	return info.bindings.filter((binding) =>
		operators.includes(binding.operator),
	);
}

export function groupParamEntries(
	parameters: readonly SqlParameterInfo[],
): HomeParamEntry[] {
	const used = new Set<string>();
	const entries: HomeParamEntry[] = [];
	for (const info of parameters) {
		if (used.has(info.name)) continue;
		const kind = rangeKind(info);
		const lower = kind ? boundOn(info, LOWER_OPERATORS) : [];
		const partner = lower.length
			? parameters.find(
					(other) =>
						other.name !== info.name &&
						!used.has(other.name) &&
						rangeKind(other) === kind &&
						boundOn(other, UPPER_OPERATORS).some((upper) =>
							lower.some(
								(low) =>
									low.table === upper.table && low.column === upper.column,
							),
						),
				)
			: undefined;
		used.add(info.name);
		if (kind && partner) {
			used.add(partner.name);
			entries.push({ type: "range", kind, from: info, to: partner });
		} else {
			entries.push({ type: "single", info });
		}
	}
	return entries;
}

export function describeBinding(info: SqlParameterInfo): string | null {
	const binding = primaryBinding(info);
	return binding ? `${binding.table}.${binding.column}` : null;
}

export function optionValues(options: SqlParameterOptions | null): {
	values: unknown[];
	truncated: boolean;
} {
	if (options && "values" in options)
		return { values: options.values, truncated: options.truncated };
	return { values: [], truncated: false };
}

export function optionRange(
	options: SqlParameterOptions | null,
): { min: unknown; max: unknown } | null {
	return options && "min" in options
		? { min: options.min, max: options.max }
		: null;
}

export function paramLabel(
	name: string,
	overrides: Record<string, HomeDataParamControl>,
): string {
	return overrides[name]?.label || name;
}

export function interactiveParamNames(
	config: Pick<HomeDataConfig, "paramControls" | "sourceKind" | "sql">,
): string[] {
	if (config.sourceKind !== "sql") return [];
	return Object.entries(config.paramControls)
		.filter(([, control]) => control.interactive)
		.map(([name]) => name);
}

export function toDateInput(value: unknown): string {
	if (typeof value !== "string") return "";
	return /^\d{4}-\d{2}-\d{2}/.test(value) ? value.slice(0, 10) : "";
}

export function toDateTimeInput(value: unknown): string {
	if (typeof value !== "string") return "";
	return /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(value) ? value.slice(0, 16) : "";
}

export function parseNumberInput(text: string): number | undefined {
	if (!text.trim()) return undefined;
	const value = Number(text);
	return Number.isFinite(value) ? value : undefined;
}

export function paramOptionKey(value: unknown): string {
	return typeof value === "string" ? value : JSON.stringify(value);
}

export function applyParamOverrides(
	config: HomeDataConfig,
	overrides: Record<string, unknown> | undefined,
): HomeDataConfig {
	const names = interactiveParamNames(config);
	if (!overrides || !names.length) return config;
	const picked = Object.fromEntries(
		names
			.filter((name) => overrides[name] !== undefined)
			.map((name) => [name, overrides[name]]),
	);
	if (!Object.keys(picked).length) return config;
	return { ...config, queryParams: { ...config.queryParams, ...picked } };
}

export function homeDataSourcePayload(
	config: Pick<HomeDataConfig, "sources">,
): CrossAppSqlSource[] {
	return config.sources.map(({ alias, appId, scope }) => ({
		alias,
		app_id: appId,
		scope,
	}));
}

export function withParamControl(
	controls: Record<string, HomeDataParamControl>,
	name: string,
	change: Partial<HomeDataParamControl>,
): Record<string, HomeDataParamControl> {
	const merged: HomeDataParamControl = { ...controls[name], ...change };
	const cleaned: HomeDataParamControl = {};
	if (merged.interactive) cleaned.interactive = true;
	if (merged.label) cleaned.label = merged.label;
	if (merged.control) cleaned.control = merged.control;
	const { [name]: _removed, ...rest } = controls;
	return Object.keys(cleaned).length ? { ...rest, [name]: cleaned } : rest;
}
