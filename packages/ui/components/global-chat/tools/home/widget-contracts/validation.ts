import { safeHomeHref } from "../../../../home/home-content/config";
import {
	HOME_DATA_SQL_MAX_CHARS,
	extractHomeQueryParameters,
	normalizeHomeDataConfig,
} from "../../../../home/home-data-query";
import { isHomeStorageImagePath } from "../../../../home/home-image-config";
import { HOME_WIDGET_CONFIG_CONTRACTS } from "./definitions";
import { APP_EMBED_RESERVED_QUERY_KEYS } from "./options";
import type {
	ContractScalar,
	HomeWidgetConfigCondition,
	HomeWidgetConfigFieldContract,
	HomeWidgetConfigIssue,
	HomeWidgetObjectContract,
	KnownHomeWidgetType,
} from "./types";

function isRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function valueAt(
	value: Record<string, unknown>,
	contract: HomeWidgetObjectContract,
	key: string,
) {
	return value[key] ?? contract.fields[key]?.default;
}

function conditionMatches(
	condition: HomeWidgetConfigCondition | undefined,
	value: Record<string, unknown>,
	contract: HomeWidgetObjectContract,
) {
	if (!condition) return true;
	if (
		condition.all &&
		!condition.all.every((entry) => conditionMatches(entry, value, contract))
	)
		return false;
	if (!condition.field) return true;
	const actual = valueAt(value, contract, condition.field);
	if (condition.equals !== undefined) return actual === condition.equals;
	if (condition.in) return condition.in.includes(actual as ContractScalar);
	if (condition.not_equals !== undefined)
		return actual !== condition.not_equals;
	if (condition.not_in)
		return !condition.not_in.includes(actual as ContractScalar);
	return true;
}

function hasValue(value: unknown) {
	return typeof value === "string"
		? Boolean(value.trim())
		: Array.isArray(value)
			? value.length > 0
			: value !== undefined && value !== null;
}

function pushIssue(
	issues: HomeWidgetConfigIssue[],
	code: string,
	path: string,
	message: string,
	severity: "error" | "warning" = "error",
) {
	issues.push({ severity, code, path, message });
}

function validateRequirements(
	value: Record<string, unknown>,
	contract: HomeWidgetObjectContract,
	path: string,
	issues: HomeWidgetConfigIssue[],
) {
	for (const requirement of contract.requirements ?? []) {
		if (!conditionMatches(requirement.when, value, contract)) continue;
		const severity = requirement.severity ?? "error";
		for (const key of requirement.required ?? []) {
			if (value[key] !== undefined) continue;
			pushIssue(
				issues,
				requirement.code,
				`${path}.${key}`,
				requirement.message,
				severity,
			);
		}
		for (const key of requirement.non_empty ?? []) {
			if (hasValue(valueAt(value, contract, key))) continue;
			pushIssue(
				issues,
				requirement.code,
				`${path}.${key}`,
				requirement.message,
				severity,
			);
		}
		if (
			requirement.any_non_empty?.length &&
			!requirement.any_non_empty.some((key) =>
				hasValue(valueAt(value, contract, key)),
			)
		)
			pushIssue(issues, requirement.code, path, requirement.message, severity);
		for (const key of requirement.positive ?? []) {
			const actual = valueAt(value, contract, key);
			if (typeof actual === "number" && Number.isFinite(actual) && actual > 0)
				continue;
			pushIssue(
				issues,
				requirement.code,
				`${path}.${key}`,
				requirement.message,
				severity,
			);
		}
		for (const key of requirement.empty ?? []) {
			if (!hasValue(valueAt(value, contract, key))) continue;
			pushIssue(
				issues,
				requirement.code,
				`${path}.${key}`,
				requirement.message,
				severity,
			);
		}
		for (const [key, expected] of Object.entries(requirement.equals ?? {})) {
			if (valueAt(value, contract, key) === expected) continue;
			pushIssue(
				issues,
				requirement.code,
				`${path}.${key}`,
				requirement.message,
				severity,
			);
		}
		for (const key of requirement.numeric_strings ?? []) {
			const actual = valueAt(value, contract, key);
			if (
				typeof actual === "string" &&
				actual.trim() &&
				Number.isFinite(Number(actual))
			)
				continue;
			pushIssue(
				issues,
				requirement.code,
				`${path}.${key}`,
				requirement.message,
				severity,
			);
		}
		for (const [key, allowed] of Object.entries(
			requirement.allowed_values ?? {},
		)) {
			if (allowed.includes(valueAt(value, contract, key) as ContractScalar))
				continue;
			pushIssue(
				issues,
				requirement.code,
				`${path}.${key}`,
				requirement.message,
				severity,
			);
		}
	}
}

function validateField(
	value: unknown,
	contract: HomeWidgetConfigFieldContract,
	path: string,
	issues: HomeWidgetConfigIssue[],
) {
	if (value === null && contract.nullable) return;
	const typeMatches =
		(contract.type === "string" && typeof value === "string") ||
		(contract.type === "number" &&
			typeof value === "number" &&
			Number.isFinite(value)) ||
		(contract.type === "boolean" && typeof value === "boolean") ||
		(contract.type === "object" && isRecord(value)) ||
		(contract.type === "string_list" &&
			Array.isArray(value) &&
			value.every((entry) => typeof entry === "string")) ||
		(contract.type === "object_list" &&
			Array.isArray(value) &&
			value.every(isRecord));
	if (!typeMatches) {
		pushIssue(
			issues,
			"home_widget_config_type_invalid",
			path,
			`Expected ${contract.type}.`,
		);
		return;
	}
	if (contract.enum && !contract.enum.includes(value as ContractScalar)) {
		const legacy = contract.legacy_values?.find(
			(entry) => entry.value === value,
		);
		if (legacy)
			pushIssue(
				issues,
				"home_widget_config_legacy_value",
				path,
				`This legacy value renders as '${legacy.maps_to}'. Use '${legacy.maps_to}' for new layouts.`,
				"warning",
			);
		else
			pushIssue(
				issues,
				"home_widget_config_option_invalid",
				path,
				`Use one of: ${contract.enum.join(", ")}.`,
			);
	}
	if (typeof value === "string") {
		if (contract.pattern && !new RegExp(contract.pattern).test(value))
			pushIssue(
				issues,
				"home_widget_config_pattern_invalid",
				path,
				`Value must match ${contract.pattern}.`,
			);
		if (
			contract.string_format === "date-time" &&
			value &&
			!Number.isFinite(new Date(value).getTime())
		)
			pushIssue(
				issues,
				"home_widget_config_date_invalid",
				path,
				"Use a valid date or date-time.",
			);
		if (
			contract.string_format === "route" &&
			value &&
			(!value.startsWith("/") || value.startsWith("//"))
		)
			pushIssue(
				issues,
				"home_widget_config_route_invalid",
				path,
				"App routes must start with one '/'.",
			);
		if (
			contract.reference?.kind === "safe_home_url" &&
			value.trim() &&
			!safeHomeHref(value)
		)
			pushIssue(
				issues,
				"unsafe_home_url",
				path,
				"Use a relative path or an http, https, mailto, or tel URL.",
			);
		if (contract.string_format === "image-url" && value.trim()) {
			const href = safeHomeHref(value);
			if (
				!href ||
				(!href.startsWith("/") &&
					!href.startsWith("http://") &&
					!href.startsWith("https://"))
			)
				pushIssue(
					issues,
					"home_widget_config_image_url_invalid",
					path,
					"Use a relative path or an http or https image URL.",
				);
		}
		if (
			contract.string_format === "storage-image-path" &&
			value.trim() &&
			!isHomeStorageImagePath(value)
		)
			pushIssue(
				issues,
				"home_widget_config_storage_image_path_invalid",
				path,
				"Use an app-relative file path without a URL scheme, leading slash, or parent-directory segments.",
			);
	}
	if (typeof value === "number") {
		if (contract.integer && !Number.isInteger(value))
			pushIssue(
				issues,
				"home_widget_config_integer_required",
				path,
				"Use an integer.",
			);
		if (contract.minimum !== undefined && value < contract.minimum)
			pushIssue(
				issues,
				"home_widget_config_number_too_small",
				path,
				`Use ${contract.minimum} or greater.`,
			);
		if (
			contract.exclusive_minimum !== undefined &&
			value <= contract.exclusive_minimum
		)
			pushIssue(
				issues,
				"home_widget_config_number_too_small",
				path,
				`Use a value greater than ${contract.exclusive_minimum}.`,
			);
		if (contract.maximum !== undefined && value > contract.maximum)
			pushIssue(
				issues,
				"home_widget_config_number_too_large",
				path,
				`Use ${contract.maximum} or less.`,
			);
		if (
			contract.ranges &&
			!contract.ranges.some(
				(range) => value >= range.minimum && value <= range.maximum,
			)
		)
			pushIssue(
				issues,
				"home_widget_config_number_out_of_range",
				path,
				"Use a supported numeric range.",
			);
	}
	if (Array.isArray(value)) {
		if (contract.min_items !== undefined && value.length < contract.min_items)
			pushIssue(
				issues,
				"home_widget_config_list_too_short",
				path,
				`Use at least ${contract.min_items} items.`,
			);
		if (contract.max_items !== undefined && value.length > contract.max_items)
			pushIssue(
				issues,
				"home_widget_config_list_too_long",
				path,
				`Use at most ${contract.max_items} items.`,
			);
		if (contract.unique_items && new Set<unknown>(value).size !== value.length)
			pushIssue(
				issues,
				"home_widget_config_list_duplicate",
				path,
				"List values must be unique.",
			);
		if (contract.item_enum) {
			value.forEach((entry, index) => {
				if (typeof entry === "string" && !contract.item_enum?.includes(entry))
					pushIssue(
						issues,
						"home_widget_config_list_option_invalid",
						`${path}[${index}]`,
						`Use one of: ${contract.item_enum?.join(", ")}.`,
					);
			});
		}
		if (contract.type === "object_list" && contract.item) {
			const uniqueBy = contract.item.unique_by;
			if (uniqueBy) {
				const seen = new Set<unknown>();
				value.forEach((entry, index) => {
					if (!isRecord(entry)) return;
					const identity = entry[uniqueBy];
					if (identity === undefined || identity === "") return;
					if (seen.has(identity))
						pushIssue(
							issues,
							"home_widget_config_item_id_duplicate",
							`${path}[${index}].${uniqueBy}`,
							`Item ${uniqueBy} values must be unique.`,
						);
					seen.add(identity);
				});
			}
			value.forEach((entry, index) => {
				if (isRecord(entry))
					validateObject(
						entry,
						contract.item as HomeWidgetObjectContract,
						`${path}[${index}]`,
						issues,
					);
			});
		}
	}
}

function validateObject(
	value: Record<string, unknown>,
	contract: HomeWidgetObjectContract,
	path: string,
	issues: HomeWidgetConfigIssue[],
) {
	if (!contract.additional_properties) {
		for (const key of Object.keys(value)) {
			if (Object.hasOwn(contract.fields, key)) continue;
			pushIssue(
				issues,
				"home_widget_config_key_unknown",
				`${path}.${key}`,
				`Config key '${key}' is not advertised by this client. Preserve it when editing an existing widget.`,
				"warning",
			);
		}
	}
	for (const key of contract.required ?? []) {
		if (value[key] !== undefined) continue;
		pushIssue(
			issues,
			"home_widget_config_field_missing",
			`${path}.${key}`,
			`Config field '${key}' is required.`,
		);
	}
	for (const [key, field] of Object.entries(contract.fields)) {
		if (value[key] === undefined) continue;
		if (!conditionMatches(field.when, value, contract)) continue;
		validateField(value[key], field, `${path}.${key}`, issues);
	}
	if (contract.unique_by) {
		// A list's item validator receives one item at a time. Uniqueness is checked by its parent.
	}
	validateRequirements(value, contract, path, issues);
}

/** Validate the config of one known Home widget without reading app or data resources. */
export function validateKnownHomeWidgetConfig(
	type: string,
	config: unknown,
	path = "$.config",
): HomeWidgetConfigIssue[] {
	if (!Object.hasOwn(HOME_WIDGET_CONFIG_CONTRACTS, type)) return [];
	if (!isRecord(config)) {
		return [
			{
				severity: "error",
				code: "home_widget_config_invalid",
				path,
				message: "Widget config must be an object.",
			},
		];
	}
	const contract = HOME_WIDGET_CONFIG_CONTRACTS[type as KnownHomeWidgetType];
	const issues: HomeWidgetConfigIssue[] = [];
	validateObject(config, contract, path, issues);

	if (type === "data") {
		const raw = config;
		const normalized = normalizeHomeDataConfig(raw);
		if (
			typeof raw.mode === "string" &&
			["aggregate", "records"].includes(raw.mode) &&
			normalized.mode !== raw.mode
		)
			pushIssue(
				issues,
				"data_mode_incompatible",
				`${path}.mode`,
				`Use mode '${normalized.mode}' with visualization '${normalized.visualization}'.`,
			);
		if (isRecord(raw.queryParams)) {
			for (const key of Object.keys(raw.queryParams)) {
				if (!key.startsWith("__home_")) continue;
				pushIssue(
					issues,
					"data_query_parameter_reserved",
					`${path}.queryParams.${key}`,
					"Query parameter names cannot start with __home_.",
				);
			}
		}
		if (normalized.sourceKind === "sql") {
			if (normalized.sql.length > HOME_DATA_SQL_MAX_CHARS)
				pushIssue(
					issues,
					"data_sql_too_long",
					`${path}.sql`,
					`SQL must be at most ${HOME_DATA_SQL_MAX_CHARS} characters.`,
				);
			for (const parameter of extractHomeQueryParameters(normalized.sql)) {
				if (!parameter.startsWith("__home_")) continue;
				pushIssue(
					issues,
					"data_query_parameter_reserved",
					`${path}.sql`,
					`SQL parameter '${parameter}' is reserved; names cannot start with __home_.`,
				);
			}
			normalized.sources.forEach((source, index) => {
				if (!["public", "information_schema"].includes(source.alias)) return;
				pushIssue(
					issues,
					"data_sql_alias_reserved",
					`${path}.sources[${index}].alias`,
					`Alias '${source.alias}' is reserved.`,
				);
			});
		}
	}

	if (type === "app-embed") {
		const querySources: Array<["query" | "route", string]> = [];
		if (typeof config.query === "string")
			querySources.push(["query", config.query.replace(/^\?/, "")]);
		if (typeof config.route === "string") {
			const queryStart = config.route.indexOf("?");
			if (queryStart >= 0)
				querySources.push(["route", config.route.slice(queryStart + 1)]);
		}
		for (const [field, query] of querySources) {
			for (const key of new URLSearchParams(query).keys()) {
				if (!APP_EMBED_RESERVED_QUERY_KEYS.has(key)) continue;
				pushIssue(
					issues,
					"app_embed_query_key_reserved",
					`${path}.${field}`,
					`Query parameter '${key}' is reserved and would be discarded.`,
				);
			}
		}
	}

	return issues;
}
