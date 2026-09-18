import { stableStringify } from "../../../../lib/stable-stringify";
import { HOME_ACCENTS } from "../../../home/home-appearance";
import {
	homeLayoutByteLength,
	minimumHomeWidgetRows,
} from "../../../home/home-layout";
import {
	homeLayoutFingerprint,
	parseHomeLayoutJson,
} from "../../../home/home-layout-json";
import type { IHomeLayout, IHomeWidget } from "../../../home/types";
import {
	HOME_VARIANTS,
	HOME_WIDGET_TYPES,
	issue,
	objectRecord,
} from "./shared";
import type { HomeLayoutValidationResult, HomeToolIssue } from "./types";
import { validateKnownHomeWidgetConfig } from "./widget-contracts/validation";

const LEGACY_VARIANTS_BY_WIDGET_TYPE = new Map<string, Set<string>>([
	[
		"app-collection",
		new Set([
			"grid",
			"standard",
			"compact",
			"list",
			"editorial",
			"icons",
			"carousel",
			"spotlight",
		]),
	],
	["packages", new Set(["grid", "standard", "compact", "featured", "list"])],
	["models", new Set(["grid", "standard", "list"])],
	["quick-links", new Set(["grid", "list"])],
]);

const SOLID_WIDGET_TYPES = new Set([
	"app-spotlight",
	"app-ranking",
	"app-collection-feature",
	"model-spotlight",
]);

const FUTURE_VALUE_ISSUES = new Set([
	"unknown_widget_variant",
	"unknown_widget_accent",
	"home_widget_config_option_invalid",
	"home_widget_config_list_option_invalid",
]);

function widgetValueAtPath(widget: IHomeWidget, path: string): unknown {
	let value: unknown = widget;
	for (const key of path.replace(/\[(\d+)\]/g, ".$1").split(".")) {
		if (!value || typeof value !== "object" || !Object.hasOwn(value, key))
			return undefined;
		value = (value as Record<string, unknown>)[key];
	}
	return value;
}

/** A newer client's enum value may survive an edit; this client cannot author it. */
function preserveFutureWidgetValues(
	issues: HomeToolIssue[],
	widget: IHomeWidget,
	index: number,
	current: IHomeWidget | undefined,
): HomeToolIssue[] {
	if (!current || current.type !== widget.type) return issues;
	const prefix = `$.widgets[${index}].`;
	return issues.map((entry) => {
		if (!FUTURE_VALUE_ISSUES.has(entry.code) || !entry.path.startsWith(prefix))
			return entry;
		const relativePath = entry.path.slice(prefix.length);
		const value = widgetValueAtPath(widget, relativePath);
		if (
			value === undefined ||
			stableStringify(value) !==
				stableStringify(widgetValueAtPath(current, relativePath))
		)
			return entry;
		return {
			...entry,
			severity: "warning",
			code: "home_widget_future_value_preserved",
			message:
				"This unsupported value is unchanged from the current widget. Preserve it exactly or choose a value advertised by this client.",
		};
	});
}

function asJsonSource(value: unknown): string | undefined {
	try {
		return JSON.stringify(value);
	} catch {
		return undefined;
	}
}

const LAYOUT_OWNED_UTILITY =
	/^(?:static|fixed|absolute|relative|sticky|z-.+|(?:col|row)-(?:span|start|end)-.+|self-.+)$/;

function classSegments(token: string): string[] {
	const segments: string[] = [];
	let depth = 0;
	let start = 0;
	for (let index = 0; index < token.length; index++) {
		const char = token[index];
		if (char === "[" || char === "(") depth++;
		else if ((char === "]" || char === ")") && depth > 0) depth--;
		else if (char === ":" && depth === 0) {
			segments.push(token.slice(start, index));
			start = index + 1;
		}
	}
	segments.push(token.slice(start));
	return segments;
}

/** Layout-owned utilities have no effect; sibling arbitrary variants select outside the widget. */
function outOfScopeClass(token: string) {
	const variants = classSegments(token);
	const utility = (variants.pop() ?? "").replace(/^!?-?/, "").replace(/!$/, "");
	return (
		LAYOUT_OWNED_UTILITY.test(utility) ||
		variants.some((variant) => variant.startsWith("[") && /[~+]/.test(variant))
	);
}

/** Normalization drops a non-string className, so the raw candidate must be checked. */
function validateRawClassNameType(
	rawWidget: unknown,
	path: string,
	issues: HomeToolIssue[],
) {
	const raw =
		objectRecord(rawWidget) && objectRecord(rawWidget.appearance)
			? rawWidget.appearance.className
			: undefined;
	if (raw === undefined || typeof raw === "string") return;
	issues.push(
		issue(
			"error",
			"home_widget_class_name_type_invalid",
			path,
			"Use a space-separated string of Tailwind classes, or omit className.",
		),
	);
}

function validateClassNameScope(
	className: string | undefined,
	path: string,
	issues: HomeToolIssue[],
) {
	const tokens = new Set(className?.split(" ").filter(outOfScopeClass));
	if (tokens.size === 0) return;
	issues.push(
		issue(
			"warning",
			"home_widget_class_name_out_of_scope",
			path,
			`These classes have no effect or reach outside the widget: ${[...tokens].join(", ")}. The layout controls position, grid span, height, and self-alignment; z-index and sibling variants (~, +) are not supported.`,
		),
	);
}

function validateWidget(
	widget: IHomeWidget,
	rawWidget: unknown,
	index: number,
	issues: HomeToolIssue[],
) {
	const path = `$.widgets[${index}]`;
	validateRawClassNameType(rawWidget, `${path}.appearance.className`, issues);
	if (!HOME_WIDGET_TYPES.has(widget.type)) {
		issues.push(
			issue(
				"warning",
				"unknown_widget_type",
				`${path}.type`,
				`Widget type '${widget.type}' is not available in this client and will render as unavailable.`,
			),
		);
		// A later client may own this widget's size, appearance, and config contracts. The
		// caller separately proves that an unsupported widget is preserved byte-for-byte.
		return;
	}
	const minimumRows = minimumHomeWidgetRows(widget);
	if (widget.size.rows < minimumRows) {
		issues.push(
			issue(
				"error",
				"widget_too_short",
				`${path}.size.rows`,
				`This widget needs at least ${minimumRows} rows.`,
			),
		);
	}
	const legacyVariants = LEGACY_VARIANTS_BY_WIDGET_TYPE.get(widget.type);
	if (
		!HOME_VARIANTS.has(widget.appearance.variant) &&
		!legacyVariants?.has(widget.appearance.variant)
	) {
		issues.push(
			issue(
				"error",
				"unknown_widget_variant",
				`${path}.appearance.variant`,
				"Use card, borderless, tinted, or solid.",
			),
		);
	} else if (
		widget.appearance.variant === "solid" &&
		!SOLID_WIDGET_TYPES.has(widget.type)
	) {
		issues.push(
			issue(
				"error",
				"unsupported_solid_variant",
				`${path}.appearance.variant`,
				"The solid surface is only supported by spotlight, ranking, and collection feature widgets.",
			),
		);
	}
	if (!Object.hasOwn(HOME_ACCENTS, widget.appearance.accent)) {
		issues.push(
			issue(
				"error",
				"unknown_widget_accent",
				`${path}.appearance.accent`,
				`Use one of: ${Object.keys(HOME_ACCENTS).join(", ")}.`,
			),
		);
	}
	validateClassNameScope(
		widget.appearance.className,
		`${path}.appearance.className`,
		issues,
	);
	issues.push(
		...validateKnownHomeWidgetConfig(
			widget.type,
			widget.config,
			`${path}.config`,
		),
	);
}

/** Validate and canonicalize an untrusted Home JSON value without reading external resources. */
export function validateHomeLayoutCandidate(
	value: unknown,
	currentLayout?: IHomeLayout,
): HomeLayoutValidationResult {
	const source = asJsonSource(value);
	if (source === undefined) {
		return {
			status: "validation_error",
			valid: false,
			issues: [
				issue(
					"error",
					"home_layout_not_json",
					"$",
					"The layout must be a JSON-serializable object.",
				),
			],
		};
	}
	const parsed = parseHomeLayoutJson(source);
	if (!parsed.ok) {
		return {
			status: "validation_error",
			valid: false,
			issues: [issue("error", "home_layout_invalid", "$", parsed.error)],
		};
	}
	const issues: HomeToolIssue[] = [];
	if (stableStringify(value) !== stableStringify(parsed.layout)) {
		issues.push(
			issue(
				"warning",
				"home_layout_normalized",
				"$",
				"Optional widget fields and bounded sizes were normalized in canonical_layout.",
			),
		);
	}
	if (parsed.layout.widgets.length === 0) {
		issues.push(
			issue(
				"warning",
				"home_layout_empty",
				"$.widgets",
				"The Home layout has no widgets.",
			),
		);
	}
	const currentById = new Map(
		currentLayout?.widgets.map((widget) => [widget.id, widget]),
	);
	const rawWidgets =
		objectRecord(value) && Array.isArray(value.widgets) ? value.widgets : [];
	parsed.layout.widgets.forEach((widget, index) => {
		const widgetIssues: HomeToolIssue[] = [];
		validateWidget(widget, rawWidgets[index], index, widgetIssues);
		issues.push(
			...preserveFutureWidgetValues(
				widgetIssues,
				widget,
				index,
				currentById.get(widget.id),
			),
		);
	});
	const valid = !issues.some((entry) => entry.severity === "error");
	return {
		status: valid ? "ok" : "validation_error",
		valid,
		issues,
		layout: parsed.layout,
		canonical_layout: parsed.layout,
		fingerprint: homeLayoutFingerprint(parsed.layout),
		byte_count: homeLayoutByteLength(parsed.layout),
	};
}

/** Merge asynchronous reference diagnostics into a local validation result. */
export function withHomeReferenceIssues(
	validation: HomeLayoutValidationResult,
	referenceIssues: HomeToolIssue[],
): HomeLayoutValidationResult {
	const issues = [...validation.issues, ...referenceIssues];
	const valid =
		Boolean(validation.layout) &&
		!issues.some((entry) => entry.severity === "error");
	return {
		...validation,
		status: valid ? "ok" : "validation_error",
		valid,
		issues,
	};
}
