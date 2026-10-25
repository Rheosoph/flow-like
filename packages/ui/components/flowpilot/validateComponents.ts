/**
 * Validates and sanitises AI-generated SurfaceComponent arrays.
 *
 * LLMs may hallucinate props, omit required fields, use bare strings
 * instead of BoundValue wrappers, or reference unknown component types.
 * This module fixes what it can and strips the rest so the A2UI renderer
 * never receives invalid data.
 */

import { lowerAuthoredComponent } from "../a2ui/authoring/lower";
import { propSpec } from "../a2ui/authoring/manifest";
import {
	type A2UIComponentType,
	COMPONENT_BASE_PROPS,
	COMPONENT_PROPS,
} from "../a2ui/component-prop-manifest";
import { getRegisteredTypes } from "../a2ui/component-type-registry";
import {
	isSemanticBoxTag,
	normalizeSemanticBoxTag,
} from "../a2ui/semantic-box-tags";
import { normalizeSurfaceComponentForPersistence } from "../a2ui/style-normalization";
import type {
	Action,
	BoundValue,
	CanvasSettings,
	Style,
	SurfaceComponent,
} from "../a2ui/types";

// ---------------------------------------------------------------------------
// Known props per component type
// ---------------------------------------------------------------------------

/** Props accepted beyond the types.ts interfaces (runtime-only wiring the
 *  renderer supports, e.g. the inline widget definition consumed by
 *  A2UIWidgetInstance). */
const RUNTIME_ONLY_PROPS: Partial<
	Record<A2UIComponentType, readonly string[]>
> = {
	widgetInstance: ["inlineWidgetDef"],
};

/** Which props a given component type accepts (excluding the shared base:
 *  `type`, `id`, `style`, `children`, `actions`, `eventHandlers`, `hidden`). Derived from the
 *  compile-time-checked manifest so it cannot drift from a2ui/types.ts. */
export const KNOWN_PROPS: Record<
	string,
	ReadonlySet<string>
> = Object.fromEntries(
	(
		Object.entries(COMPONENT_PROPS) as [A2UIComponentType, readonly string[]][]
	).map(([type, props]) => [
		type,
		new Set([...props, ...(RUNTIME_ONLY_PROPS[type] ?? [])]),
	]),
);

/** Structured props the generated manifest does not describe keep their value verbatim. */
const UNDESCRIBED_STRUCTURED_PROPS: ReadonlySet<string> = new Set([
	"tabs",
	"items",
	"overlays",
	"columns",
	"data",
	"boxes",
	"hotspots",
	"markers",
	"choices",
]);

/** Shared base props all component types have. */
export const BASE_PROPS = new Set<string>(["type", ...COMPONENT_BASE_PROPS]);

/** Required props that MUST exist (non-optional in the TS interface). */
const REQUIRED_PROPS: Record<string, string[]> = {
	overlay: ["baseComponentId", "overlays"],
	text: ["content"],
	image: ["src"],
	icon: ["name"],
	video: ["src"],
	lottie: ["src"],
	fileEditor: ["editor"],
	markdown: ["content"],
	diffView: ["original", "modified"],
	badge: ["content"],
	userProfile: ["value"],
	progress: ["value"],
	button: ["label"],
	textField: ["value"],
	select: ["value", "options"],
	slider: ["value"],
	checkbox: ["checked"],
	switch: ["checked"],
	radioGroup: ["value", "options"],
	dateTimeInput: ["value"],
	fileInput: ["value"],
	imageInput: ["value"],
	voiceInput: ["value"],
	link: ["href"],
	modal: ["open"],
	tabs: ["value", "tabs"],
	accordion: ["items"],
	drawer: ["open"],
	tooltip: ["content"],
	popover: ["contentComponentId"],
	table: ["columns", "data"],
	tableRow: ["cells"],
	tableCell: ["content"],
	canvas2d: ["width", "height"],
	sprite: ["src", "x", "y"],
	shape: ["shapeType", "x", "y"],
	scene3d: ["width", "height"],
	model3d: ["src"],
	dialogue: ["text"],
	characterPortrait: ["image"],
	choiceMenu: ["choices"],
	inventoryGrid: ["items"],
	healthBar: ["value", "maxValue"],
	miniMap: ["width", "height"],
	aspectRatio: ["ratio"],
	nivoChart: ["chartType"],
	forecastChart: ["forecastId"],
	boundingBoxOverlay: ["src", "boxes"],
	imageLabeler: ["src", "labels"],
	imageHotspot: ["src", "hotspots"],
	widgetInstance: ["instanceId", "widgetId"],
	calendar: ["events"],
	gantt: ["tasks"],
	graph: ["nodes"],
	ontologyGraph: ["ontologyId"],
};

/** Default BoundValue to inject when a required prop is missing. */
const DEFAULT_BOUND_VALUES: Record<string, BoundValue> = {
	content: { literalString: "" },
	src: { literalString: "" },
	editor: { literalString: "wordcraft" },
	original: { literalString: "" },
	modified: { literalString: "" },
	name: { literalString: "circle" },
	value: { literalString: "" },
	options: { literalOptions: [] },
	checked: { literalBool: false },
	open: { literalBool: false },
	label: { literalString: "" },
	href: { literalString: "#" },
	width: { literalNumber: 300 },
	height: { literalNumber: 200 },
	x: { literalNumber: 0 },
	y: { literalNumber: 0 },
	shapeType: { literalString: "rectangle" },
	ratio: { literalNumber: 1 },
	text: { literalString: "" },
	maxValue: { literalNumber: 100 },
	chartType: { literalString: "bar" },
	ontologyId: { literalString: "" },
};

/** Required structured props that default to an empty array of items. */
const EMPTY_LIST_DEFAULTS: ReadonlySet<string> = new Set([
	"overlay.overlays",
	"tabs.tabs",
	"accordion.items",
]);

/** Required bound props that default to an empty JSON list. */
const EMPTY_JSON_LIST_DEFAULTS: ReadonlySet<string> = new Set([
	"columns",
	"data",
	"cells",
	"choices",
	"items",
	"boxes",
	"labels",
	"hotspots",
	"nodes",
]);

function defaultRequiredProp(type: string, prop: string): unknown {
	if (EMPTY_LIST_DEFAULTS.has(`${type}.${prop}`)) return [];
	if (EMPTY_JSON_LIST_DEFAULTS.has(prop)) {
		return { literalJson: "[]" } satisfies BoundValue;
	}
	return DEFAULT_BOUND_VALUES[prop];
}

const MAX_COMPONENTS = 120;
const MAX_COMPONENT_ID_CHARS = 120;
const MAX_BOUND_STRING_CHARS = 8_000;
const MAX_EVENT_HANDLERS = 64;
const MAX_EVENT_NAME_CHARS = 128;
const MAX_ACTIONS_PER_EVENT = 20;

// ---------------------------------------------------------------------------
// BoundValue coercion
// ---------------------------------------------------------------------------

function isBoundValue(v: unknown): v is BoundValue {
	if (v == null || typeof v !== "object") return false;
	const obj = v as Record<string, unknown>;
	return (
		"literalString" in obj ||
		"literalNumber" in obj ||
		"literalBool" in obj ||
		"literalOptions" in obj ||
		"literalJson" in obj ||
		"path" in obj
	);
}

function clipString(value: string, maxChars: number): string {
	return value.length > maxChars ? value.slice(0, maxChars) : value;
}

function sanitizeBoundValue(value: BoundValue): BoundValue {
	if ("literalString" in value) {
		return {
			literalString: clipString(value.literalString, MAX_BOUND_STRING_CHARS),
		};
	}
	if ("literalJson" in value) {
		return {
			literalJson: clipString(value.literalJson, MAX_BOUND_STRING_CHARS),
		};
	}
	if ("path" in value) {
		return {
			...value,
			path: clipString(value.path, 512),
			defaultValue:
				typeof value.defaultValue === "string"
					? clipString(value.defaultValue, MAX_BOUND_STRING_CHARS)
					: value.defaultValue,
		};
	}
	if ("literalOptions" in value) {
		return {
			literalOptions: value.literalOptions.slice(0, 100).map((option) => ({
				value: clipString(String(option.value), 256),
				label: clipString(String(option.label), 512),
			})),
		};
	}
	return value;
}

/** Wrap a bare primitive as a BoundValue. */
function coerceToBoundValue(v: unknown): BoundValue | undefined {
	if (v == null) return undefined;
	if (isBoundValue(v)) return sanitizeBoundValue(v as BoundValue);
	if (typeof v === "string")
		return { literalString: clipString(v, MAX_BOUND_STRING_CHARS) };
	if (typeof v === "number") return { literalNumber: v };
	if (typeof v === "boolean") return { literalBool: v };
	if (Array.isArray(v)) {
		// Could be options array [{value, label}]
		if (
			v.length > 0 &&
			typeof v[0] === "object" &&
			v[0] !== null &&
			"value" in v[0]
		) {
			return sanitizeBoundValue({
				literalOptions: v as { value: string; label: string }[],
			});
		}
		return {
			literalJson: clipString(JSON.stringify(v), MAX_BOUND_STRING_CHARS),
		};
	}
	if (typeof v === "object") {
		return {
			literalJson: clipString(JSON.stringify(v), MAX_BOUND_STRING_CHARS),
		};
	}
	return undefined;
}

function sanitizeIframeSandbox(value: unknown): BoundValue | undefined {
	const boundValue = coerceToBoundValue(value);
	if (!boundValue || !("literalString" in boundValue)) return undefined;

	const allowedTokens = new Set([
		"allow-downloads",
		"allow-forms",
		"allow-modals",
		"allow-popups",
		"allow-presentation",
		"allow-scripts",
	]);
	const tokens = boundValue.literalString
		.split(/\s+/)
		.filter((token) => allowedTokens.has(token));

	return { literalString: tokens.join(" ") };
}

// ---------------------------------------------------------------------------
// Children validation
// ---------------------------------------------------------------------------

function validateChildren(
	children: unknown,
	isKnownId: (id: string) => boolean,
): { explicitList: string[] } | { template: unknown } | undefined {
	if (children == null || typeof children !== "object") return undefined;

	const c = children as Record<string, unknown>;

	if ("explicitList" in c && Array.isArray(c.explicitList)) {
		const filtered = (c.explicitList as unknown[]).filter(
			(id): id is string => typeof id === "string" && isKnownId(id),
		);
		return filtered.length > 0 ? { explicitList: filtered } : undefined;
	}

	if ("template" in c && c.template && typeof c.template === "object") {
		const template = c.template as Record<string, unknown>;
		if (
			typeof template.templateComponentId === "string" &&
			isKnownId(template.templateComponentId) &&
			typeof template.dataPath === "string"
		) {
			return {
				template: {
					templateComponentId: template.templateComponentId,
					dataPath: clipString(template.dataPath, 512),
					...(typeof template.itemIdPath === "string"
						? { itemIdPath: clipString(template.itemIdPath, 512) }
						: {}),
				},
			};
		}
	}

	return undefined;
}

// ---------------------------------------------------------------------------
// Named event handler validation
// ---------------------------------------------------------------------------

function isPlainObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * The action names `ActionHandler.tsx` dispatches. Anything else falls through its `default`
 * branch into a no-op `userAction`, so the control renders but never runs. Keep in sync with
 * `executeAction` there and with `BUILTIN_ACTION_NAMES` in the Rust emit_ui validator
 * (apps/desktop/src-tauri/src/functions/ai/copilot_sdk_tools.rs).
 */
export const BUILTIN_ACTION_NAMES = [
	"workflow_event",
	"widget_event",
	"navigate_page",
	"external_link",
	"navigate_app_config",
	"navigate_app_overview",
	"submit_feedback",
] as const;

const BUILTIN_ACTION_NAME_SET: ReadonlySet<string> = new Set(
	BUILTIN_ACTION_NAMES,
);

/** Routing actions that are inert without their target id. */
const REQUIRED_ACTION_CONTEXT_KEY: Readonly<Record<string, string>> = {
	workflow_event: "nodeId",
	widget_event: "actionId",
};

/**
 * Warn about actions the runtime cannot dispatch. This is deliberately non-destructive: the
 * action is kept so the builder's inspector can still show it as `Custom: <name>` for a human to
 * repair, while the warning travels back to the model as a tool result so it self-corrects.
 */
function warnUnroutableAction(
	action: Action,
	componentId: string,
	path: string,
	warnings: string[],
): void {
	if (!BUILTIN_ACTION_NAME_SET.has(action.name)) {
		warnings.push(
			`${componentId}: ${path}.name "${action.name}" is not a built-in action and will not run. The name is a fixed verb, never a board node / event / widget action id — put that id in the context. Valid: ${BUILTIN_ACTION_NAMES.join(", ")}.`,
		);
		return;
	}

	const requiredKey = REQUIRED_ACTION_CONTEXT_KEY[action.name];
	if (!requiredKey) return;
	const target = action.context?.[requiredKey];
	if (typeof target !== "string" || target.trim().length === 0) {
		warnings.push(
			`${componentId}: ${path} is a "${action.name}" action without context.${requiredKey}, so it renders a control that does nothing.`,
		);
	}
}

/**
 * Legacy `actions` arrive normalized by the authoring lowering (a bare name becomes
 * `{name, context: {}}`, entries without a name drop) and are otherwise kept as written. Only named
 * actions the runtime cannot route are reported, and even those are kept: dropping one would turn a
 * mis-wired control into a silently action-less one and lose the evidence a human needs to repair it.
 */
function reportLegacyActions(
	value: unknown,
	componentId: string,
	warnings: string[],
): void {
	if (!Array.isArray(value)) return;
	for (const [index, rawAction] of value.entries()) {
		if (!isPlainObject(rawAction) || typeof rawAction.name !== "string") {
			continue;
		}
		warnUnroutableAction(
			{
				name: rawAction.name,
				context: isPlainObject(rawAction.context) ? rawAction.context : {},
			},
			componentId,
			`actions[${index}]`,
			warnings,
		);
	}
}

function sanitizeEventHandlers(
	value: unknown,
	componentId: string,
	warnings: string[],
): Record<string, Action[]> | undefined {
	if (!isPlainObject(value)) {
		warnings.push(`${componentId}: removed invalid eventHandlers map`);
		return undefined;
	}

	const entries = Object.entries(value);
	if (entries.length > MAX_EVENT_HANDLERS) {
		warnings.push(
			`${componentId}: only the first ${MAX_EVENT_HANDLERS} event handlers were kept`,
		);
	}

	const sanitized: [string, Action[]][] = [];
	for (const [eventName, rawActions] of entries.slice(0, MAX_EVENT_HANDLERS)) {
		if (
			eventName.trim().length === 0 ||
			eventName.length > MAX_EVENT_NAME_CHARS
		) {
			warnings.push(`${componentId}: removed invalid event handler name`);
			continue;
		}
		if (!Array.isArray(rawActions)) {
			warnings.push(
				`${componentId}: eventHandlers.${eventName} must be an action array`,
			);
			continue;
		}
		if (rawActions.length > MAX_ACTIONS_PER_EVENT) {
			warnings.push(
				`${componentId}: eventHandlers.${eventName} was limited to ${MAX_ACTIONS_PER_EVENT} actions`,
			);
		}

		const actions: Action[] = [];
		for (const [index, rawAction] of rawActions
			.slice(0, MAX_ACTIONS_PER_EVENT)
			.entries()) {
			if (!isPlainObject(rawAction)) {
				warnings.push(
					`${componentId}: removed invalid eventHandlers.${eventName}[${index}]`,
				);
				continue;
			}

			const name = rawAction.name;
			const context = rawAction.context;
			if (typeof name !== "string" || name.trim().length === 0) {
				warnings.push(
					`${componentId}: eventHandlers.${eventName}[${index}].name must be a non-empty string`,
				);
				continue;
			}
			if (!isPlainObject(context)) {
				warnings.push(
					`${componentId}: eventHandlers.${eventName}[${index}].context must be an object`,
				);
				continue;
			}

			const action: Action = { name, context: { ...context } };
			warnUnroutableAction(
				action,
				componentId,
				`eventHandlers.${eventName}[${index}]`,
				warnings,
			);
			actions.push(action);
		}

		// An authored empty list explicitly suppresses legacy fallback. Do not turn
		// a non-empty but wholly invalid list into that meaningful state.
		if (rawActions.length === 0 || actions.length > 0) {
			sanitized.push([eventName, actions]);
		}
	}

	return Object.fromEntries(sanitized);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface ValidationResult {
	components: SurfaceComponent[];
	warnings: string[];
}

export interface SanitizeComponentOptions {
	/** Child ids that survive; every id by default, for callers that prune against their own set. */
	readonly isKnownId?: (id: string) => boolean;
	/** Accepted component types; the registered renderer types by default. */
	readonly registeredTypes?: ReadonlySet<string>;
}

const acceptAnyId = () => true;

/** The trimmed id an authored component (flat or canonical) carries. */
export function authoredComponentId(raw: unknown) {
	if (!isPlainObject(raw) || typeof raw.id !== "string") return undefined;
	const id = raw.id.trim();
	return id.length > 0 ? id : undefined;
}

function sanitizeBoxTag(
	value: unknown,
	componentId: string,
	warnings: string[],
): BoundValue {
	const boxTag = coerceToBoundValue(value);
	// A path keeps its binding: A2UIBox applies the same allowlist after resolving it.
	if (boxTag && "path" in boxTag) return boxTag;
	if (
		boxTag &&
		"literalString" in boxTag &&
		isSemanticBoxTag(boxTag.literalString)
	) {
		return boxTag;
	}
	const unsafeTag =
		boxTag && "literalString" in boxTag ? ` "${boxTag.literalString}"` : "";
	warnings.push(
		`${componentId}: replaced unsafe box tag${unsafeTag} with "div"`,
	);
	return { literalString: normalizeSemanticBoxTag(undefined) };
}

/** One declared prop of a lowered component; `undefined` drops it. */
function sanitizeKnownProp(
	type: string,
	key: string,
	value: unknown,
	componentId: string,
	warnings: string[],
): unknown {
	// Widget-instance props are raw wiring data (ids, the inline widget definition,
	// per-instance param/action values), and a package widget's contract/props/bundleHash
	// must survive byte-for-byte to render.
	if (type === "widgetInstance" || type === "microWidgetInstance") return value;
	if (type === "markdown" && key === "allowHtml") {
		warnings.push(`${componentId}: disabled HTML rendering for markdown`);
		return { literalBool: false };
	}
	if (type === "iframe" && key === "sandbox")
		return sanitizeIframeSandbox(value);
	if (type === "box" && key === "as") {
		return sanitizeBoxTag(value, componentId, warnings);
	}
	// Lowering already shaped every prop the manifest describes: bound values only need
	// clipping, everything else is kept as lowered.
	const spec = propSpec(type, key);
	if (spec) {
		return spec.kind === "bound" ? (coerceToBoundValue(value) ?? value) : value;
	}
	return UNDESCRIBED_STRUCTURED_PROPS.has(key)
		? value
		: (coerceToBoundValue(value) ?? value);
}

/**
 * Sanitise one AI-authored component, flat plain-value form or canonical:
 * lowers it, strips unknown props and types, coerces bound values and injects
 * defaults for required props. Child references are validated structurally and
 * kept when `isKnownId` accepts them. Returns null when the component is skipped.
 */
export function sanitizeComponent(
	raw: unknown,
	warnings: string[],
	options: SanitizeComponentOptions = {},
): SurfaceComponent | null {
	const componentId = authoredComponentId(raw);
	if (!componentId) {
		warnings.push("Skipped component with missing id");
		return null;
	}
	if (componentId.length > MAX_COMPONENT_ID_CHARS) {
		warnings.push(`${componentId.slice(0, 32)}: component id is too long`);
		return null;
	}
	const lowered = lowerAuthoredComponent(raw);
	if (!lowered.ok) {
		warnings.push(`${componentId}: ${lowered.error}`);
		return null;
	}
	const rawComponent = lowered.component.component;
	const type = rawComponent.type as string;
	const registeredTypes =
		options.registeredTypes ?? new Set(getRegisteredTypes());
	if (!registeredTypes.has(type)) {
		warnings.push(`${componentId}: unknown component type "${type}"`);
		return null;
	}

	const knownForType = KNOWN_PROPS[type] ?? new Set<string>();
	const cleaned: Record<string, unknown> = { type };
	for (const [key, value] of Object.entries(rawComponent)) {
		if (key === "type" || key === "id" || key === "children") continue;
		if (BASE_PROPS.has(key)) {
			// Legacy actions pass through verbatim. Named handlers are additive but
			// structurally validated before they can suppress the legacy fallback.
			if (key === "eventHandlers") {
				const handlers = sanitizeEventHandlers(value, componentId, warnings);
				if (handlers !== undefined) cleaned[key] = handlers;
			} else {
				if (key === "actions") {
					reportLegacyActions(value, componentId, warnings);
				}
				cleaned[key] = value;
			}
			continue;
		}
		if (!knownForType.has(key)) {
			warnings.push(
				`${componentId}: stripped unknown prop "${key}" on ${type}`,
			);
			continue;
		}
		const sanitized = sanitizeKnownProp(
			type,
			key,
			value,
			componentId,
			warnings,
		);
		if (sanitized !== undefined) cleaned[key] = sanitized;
	}

	const required = REQUIRED_PROPS[type];
	let missingUnfixableRequiredProp = false;
	for (const prop of required ?? []) {
		if (prop in cleaned) continue;
		const defaultVal = defaultRequiredProp(type, prop);
		if (defaultVal !== undefined) {
			cleaned[prop] = defaultVal;
			warnings.push(
				`${componentId}: injected default for required prop "${prop}" on ${type}`,
			);
		} else {
			missingUnfixableRequiredProp = true;
			warnings.push(
				`${componentId}: skipped ${type} because required prop "${prop}" is missing`,
			);
		}
	}
	if (missingUnfixableRequiredProp) return null;

	const rawChildren = rawComponent.children;
	const validatedChildren = validateChildren(
		rawChildren,
		options.isKnownId ?? acceptAnyId,
	);
	if (validatedChildren) {
		cleaned.children = validatedChildren;
	} else if (rawChildren != null) {
		warnings.push(`${componentId}: removed invalid children reference`);
	}

	return normalizeSurfaceComponentForPersistence({
		id: componentId,
		style: lowered.component.style as Style | undefined,
		component: cleaned as unknown as SurfaceComponent["component"],
	});
}

/**
 * Validate and sanitise an array of AI-generated components (see
 * {@link sanitizeComponent}): skips duplicate ids and prunes child references
 * to components that were skipped.
 */
export function validateComponents(
	raw: readonly unknown[],
	canvasSettings?: CanvasSettings,
): ValidationResult {
	const warnings: string[] = [];
	const registeredTypes: ReadonlySet<string> = new Set(getRegisteredTypes());
	const input = raw.slice(0, MAX_COMPONENTS);
	if (raw.length > MAX_COMPONENTS) {
		warnings.push(`Only the first ${MAX_COMPONENTS} components were kept`);
	}

	const authoredIds = new Set(
		input
			.map(authoredComponentId)
			.filter((id): id is string => id !== undefined),
	);
	const isKnownId = (id: string) => authoredIds.has(id);
	const seenIds = new Set<string>();
	const validated: SurfaceComponent[] = [];

	for (const item of input) {
		const componentId = authoredComponentId(item);
		if (componentId && componentId.length <= MAX_COMPONENT_ID_CHARS) {
			if (seenIds.has(componentId)) {
				warnings.push(`${componentId}: skipped duplicate component id`);
				continue;
			}
			seenIds.add(componentId);
		}
		const component = sanitizeComponent(item, warnings, {
			isKnownId,
			registeredTypes,
		});
		if (component) validated.push(component);
	}

	const validIds = new Set(validated.map((component) => component.id));
	const isValidId = (id: string) => validIds.has(id);
	for (const comp of validated) {
		const component = comp.component as unknown as Record<string, unknown>;
		const validatedChildren = validateChildren(component.children, isValidId);
		if (validatedChildren) {
			component.children = validatedChildren;
		} else if (component.children != null) {
			component.children = undefined;
			warnings.push(
				`${comp.id}: removed child references to skipped components`,
			);
		}
	}

	return { components: validated, warnings };
}

/**
 * Hard ceiling on a surface stylesheet, mirrored by the emit_ui validator in
 * `copilot_sdk_tools.rs`. Sized well above a complete design system so it only
 * catches runaway output.
 */
export const MAX_CUSTOM_CSS_CHARS = 40_000;

/**
 * Validate canvas settings from AI output.
 * Strips unknown keys and ensures values are sensible.
 */
export function validateCanvasSettings(
	raw: unknown,
	warnings?: string[],
): CanvasSettings | undefined {
	if (!raw || typeof raw !== "object") return undefined;
	const obj = raw as Record<string, unknown>;

	const result: CanvasSettings = {};

	if (typeof obj.backgroundColor === "string") {
		result.backgroundColor = obj.backgroundColor;
	}
	if (typeof obj.padding === "string") {
		result.padding = obj.padding;
	}
	if (typeof obj.customCss === "string") {
		// An oversized sheet is dropped whole rather than cut: CSS cannot be
		// truncated safely at an arbitrary character boundary, and omitting the
		// field keeps the surface's previous stylesheet. Everything under the cap
		// is parsed off-thread by ScopedCustomCss.
		if (obj.customCss.length > MAX_CUSTOM_CSS_CHARS) {
			warnings?.push(
				`canvasSettings.customCss was not applied: ${obj.customCss.length} characters exceeds the ${MAX_CUSTOM_CSS_CHARS} character limit. The previous stylesheet is unchanged — consolidate the rules and send a complete sheet under the limit.`,
			);
		} else {
			result.customCss = obj.customCss;
		}
	}
	if (typeof obj.backgroundImage === "string") {
		const image = obj.backgroundImage.trim();
		if (
			/^(https?:\/\/|data:image\/(?:png|jpeg|webp|gif);base64,)/i.test(image)
		) {
			result.backgroundImage = image;
		}
	}

	return Object.keys(result).length > 0 ? result : undefined;
}
