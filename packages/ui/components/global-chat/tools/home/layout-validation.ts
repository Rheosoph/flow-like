import postcss, { type Root, type Rule } from "postcss";
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

const ELEMENT_VARIANTS = new Set([
	"before",
	"after",
	"first-letter",
	"first-line",
	"marker",
	"selection",
	"placeholder",
	"file",
	"backdrop",
	"*",
	"**",
]);

function targetsOtherElement(variant: string) {
	if (ELEMENT_VARIANTS.has(variant)) return true;
	return variant.startsWith("[&") && /[_>]|::/.test(variant.slice(2));
}

/** Layout-owned utilities do nothing on the surface; fixed and sibling variants leave the widget. */
function outOfScopeClass(token: string) {
	const variants = classSegments(token);
	const utility = (variants.pop() ?? "").replace(/^!?-?/, "").replace(/!$/, "");
	return (
		utility === "fixed" ||
		(LAYOUT_OWNED_UTILITY.test(utility) &&
			!variants.some(targetsOtherElement)) ||
		variants.some((variant) => variant.startsWith("[") && /[~+]/.test(variant))
	);
}

const RAW_STYLING_FIELDS = [
	{
		field: "className",
		code: "home_widget_class_name_type_invalid",
		message:
			"Use a string of space-separated Tailwind CSS v4 classes, or omit className.",
	},
	{
		field: "css",
		code: "home_widget_css_type_invalid",
		message: "Use a plain CSS stylesheet string, or omit css.",
	},
] as const;

/** Normalization drops non-string styling fields, so the raw candidate must be checked. */
function validateRawStylingTypes(
	rawWidget: unknown,
	path: string,
	issues: HomeToolIssue[],
) {
	const appearance =
		objectRecord(rawWidget) && objectRecord(rawWidget.appearance)
			? rawWidget.appearance
			: undefined;
	for (const { field, code, message } of RAW_STYLING_FIELDS) {
		const raw = appearance?.[field];
		if (raw === undefined || typeof raw === "string") continue;
		issues.push(issue("error", code, `${path}.appearance.${field}`, message));
	}
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
			`These classes have no effect or reach outside the widget: ${[...tokens].join(", ")}. The layout owns the surface's position, grid span, height and self-alignment, so absolute, relative, sticky, static, z-*, col-*/row-* span/start/end and self-* do nothing on the surface; use them only behind variants that target other elements, such as before:, after:, *: or [&_span]:. fixed escapes the widget with or without a variant, and sibling variants (~, +) reach neighboring widgets.`,
		),
	);
}

const GLOBAL_CSS_AT_RULE =
	/^(?:property|font-face|counter-style|font-palette-values|page|import)$/i;
const LAYOUT_OWNED_CSS_PROPERTY =
	/^(?:position|inset(?:-.+)?|top|right|bottom|left|z-index|grid-(?:area|column|row)(?:-.+)?|(?:align|justify|place)-self|order|(?:min-|max-)?(?:width|height))$/i;
// Mirrors safeScopedCss: :root always, html and body only as a bare or compound selector.
const WIDGET_ROOT_SELECTOR = /^(?::root|(?:html|body)(?=$|[.#:[]))/;

const LEGACY_PSEUDO_ELEMENT =
	/^:(?:before|after|first-line|first-letter)(?![\w-])/i;

/** Indexes of selector characters outside brackets, parentheses, strings, and escapes. */
function* topLevelIndexes(selector: string) {
	let depth = 0;
	let quote = "";
	for (let index = 0; index < selector.length; index++) {
		const char = selector[index];
		if (char === "\\") {
			index++;
			continue;
		}
		if (quote) {
			if (char === quote) quote = "";
			continue;
		}
		if (char === '"' || char === "'") quote = char;
		else if (char === "(" || char === "[") depth++;
		else if (char === ")" || char === "]") depth--;
		else if (depth === 0) yield index;
	}
}

/** First combinator after the leading compound selector. */
function leadingCombinator(selector: string) {
	for (const index of topLevelIndexes(selector)) {
		if (!/[\s>+~]/.test(selector[index])) continue;
		const next = selector.slice(index).trimStart()[0];
		return next === "+" || next === "~" || next === ">" ? next : " ";
	}
	return null;
}

function hasPseudoElement(selector: string) {
	for (const index of topLevelIndexes(selector)) {
		if (selector[index] !== ":") continue;
		if (
			selector[index + 1] === ":" ||
			LEGACY_PSEUDO_ELEMENT.test(selector.slice(index))
		)
			return true;
	}
	return false;
}

/** :root::before and :root::after are decoration layers inside the clipped surface. */
function targetsWidgetSurface(selector: string) {
	return (
		WIDGET_ROOT_SELECTOR.test(selector) &&
		!leadingCombinator(selector) &&
		!hasPseudoElement(selector)
	);
}

function isWidgetSurfaceRule(rule: Rule) {
	return rule.selectors.some((selector) =>
		targetsWidgetSurface(selector.trim()),
	);
}

/** Selectors that start at the widget surface and step to its siblings leave the widget. */
function reachesSiblingWidgets(rule: Rule) {
	const parent = rule.parent;
	const nested = parent?.type === "rule" && isWidgetSurfaceRule(parent as Rule);
	return rule.selectors.some((raw) => {
		const selector = raw.trim();
		if (nested && /^[+~]/.test(selector)) return true;
		const fromRoot =
			WIDGET_ROOT_SELECTOR.test(selector) ||
			(nested && selector.startsWith("&"));
		const combinator = fromRoot ? leadingCombinator(selector) : null;
		return combinator === "+" || combinator === "~";
	});
}

function cssScopeProblems(root: Root) {
	const problems = new Set<string>();
	root.walkAtRules((atRule) => {
		if (GLOBAL_CSS_AT_RULE.test(atRule.name)) problems.add(`@${atRule.name}`);
	});
	root.walkRules((rule) => {
		if (reachesSiblingWidgets(rule)) problems.add(rule.selector.trim());
		if (!isWidgetSurfaceRule(rule)) return;
		rule.each((node) => {
			if (node.type === "decl" && LAYOUT_OWNED_CSS_PROPERTY.test(node.prop))
				problems.add(`${node.prop} on :root`);
		});
	});
	root.walkDecls(/^position$/i, (declaration) => {
		if (/^fixed$/i.test(declaration.value.trim()))
			problems.add("position: fixed");
	});
	return [...problems];
}

const TAILWIND_AT_RULE =
	/^(?:apply|tailwind|theme|variant|custom-variant|utility|config|plugin|source|reference|screen)$/i;
const TAILWIND_FUNCTION = /(?<![\w-])(theme|--alpha|--spacing)\(/gi;

function tailwindSyntax(root: Root) {
	const found = new Set<string>();
	const addFunctions = (value: string) => {
		for (const [, name] of value.matchAll(TAILWIND_FUNCTION))
			found.add(`${name.toLowerCase()}()`);
	};
	root.walkAtRules((atRule) => {
		if (TAILWIND_AT_RULE.test(atRule.name))
			found.add(`@${atRule.name.toLowerCase()}`);
		addFunctions(atRule.params);
	});
	root.walkDecls((declaration) => addFunctions(declaration.value));
	return [...found];
}

const WRAPPED_COLOR_VARIABLE =
	/\b(hsla?|rgba?|oklch|oklab|lab|lch)\(\s*var\(\s*(--[\w-]+)/gi;

function wrappedColorVariables(root: Root) {
	const found = new Set<string>();
	root.walkDecls((declaration) => {
		for (const [, color, variable] of declaration.value.matchAll(
			WRAPPED_COLOR_VARIABLE,
		))
			found.add(`${color.toLowerCase()}(var(${variable}))`);
	});
	return [...found];
}

const CSS_CHECKS = [
	{
		code: "home_widget_css_out_of_scope",
		find: cssScopeProblems,
		message: (found: string) =>
			`These parts of the CSS reach outside the widget or fight the layout: ${found}. + and ~ after :root style neighboring widgets; @property, @font-face, @counter-style and @page are document-wide and @import is removed; the layout owns position, inset, z-index, grid placement, order, width and height on :root itself, so put them on :root::before, :root::after or inner elements; position: fixed escapes the widget from any rule.`,
	},
	{
		code: "home_widget_css_tailwind_syntax",
		find: tailwindSyntax,
		message: (found: string) =>
			`This CSS uses Tailwind-only syntax that does nothing here: ${found}. appearance.css is plain CSS; put utilities in appearance.className and use var(--primary) or color-mix() for colors.`,
	},
	{
		code: "home_widget_css_wrapped_color_variable",
		find: wrappedColorVariables,
		message: (found: string) =>
			`These declarations wrap a color variable in a color function: ${found}. Theme and --home-* variables already hold complete colors, so the wrapped value is invalid; use var(--primary) directly or color-mix(in oklab, var(--primary) 30%, transparent).`,
	},
] as const;

function validateWidgetCss(
	css: string | undefined,
	path: string,
	issues: HomeToolIssue[],
) {
	if (!css) return;
	let root: Root;
	try {
		root = postcss.parse(css);
	} catch (error) {
		issues.push(
			issue(
				"warning",
				"home_widget_css_invalid",
				path,
				`This CSS does not parse (${error instanceof Error ? error.message : "syntax error"}). Only complete rules outside the broken block will apply.`,
			),
		);
		return;
	}
	for (const { code, find, message } of CSS_CHECKS) {
		const found = find(root);
		if (found.length > 0)
			issues.push(issue("warning", code, path, message(found.join(", "))));
	}
}

function removedStylingIssues(
	widget: IHomeWidget,
	index: number,
	current: IHomeWidget | undefined,
): HomeToolIssue[] {
	if (!current) return [];
	return RAW_STYLING_FIELDS.filter(
		({ field }) =>
			current.appearance[field]?.trim() && !widget.appearance[field]?.trim(),
	).map(({ field }) =>
		issue(
			"warning",
			"home_widget_styling_removed",
			`$.widgets[${index}].appearance.${field}`,
			`The current widget has appearance.${field} and this layout drops it. Copy the current value unless the user asked to remove that styling.`,
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
	validateRawStylingTypes(rawWidget, path, issues);
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
	validateWidgetCss(widget.appearance.css, `${path}.appearance.css`, issues);
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
		const current = currentById.get(widget.id);
		validateWidget(widget, rawWidgets[index], index, widgetIssues);
		issues.push(
			...preserveFutureWidgetValues(widgetIssues, widget, index, current),
			...removedStylingIssues(widget, index, current),
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
