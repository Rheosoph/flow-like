/**
 * Validates and sanitises AI-generated SurfaceComponent arrays.
 *
 * LLMs may hallucinate props, omit required fields, use bare strings
 * instead of BoundValue wrappers, or reference unknown component types.
 * This module fixes what it can and strips the rest so the A2UI renderer
 * never receives invalid data.
 */

import { getRegisteredTypes } from "../a2ui/ComponentRegistry";
import {
	type A2UIComponentType,
	COMPONENT_BASE_PROPS,
	COMPONENT_PROPS,
} from "../a2ui/component-prop-manifest";
import {
	isSemanticBoxTag,
	normalizeSemanticBoxTag,
} from "../a2ui/semantic-box-tags";
import { normalizeSurfaceComponentForPersistence } from "../a2ui/style-normalization";
import type {
	Action,
	BoundValue,
	CanvasSettings,
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

/** Props that are plain values in types.ts (NOT BoundValue-wrapped) and must
 *  never be coerced — the renderers read them verbatim. */
const PLAIN_PROPS: Partial<Record<A2UIComponentType, ReadonlySet<string>>> = {
	overlay: new Set(["baseComponentId"]),
	popover: new Set(["contentComponentId"]),
	tabs: new Set(["listStyle", "triggerStyle", "contentStyle"]),
	link: new Set(["external", "target", "variant", "underline"]),
	plotlyChart: new Set(["series", "xAxis", "yAxis"]),
};

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
	boundingBoxOverlay: ["src", "boxes"],
	imageLabeler: ["src", "labels"],
	imageHotspot: ["src", "hotspots"],
	widgetInstance: ["instanceId", "widgetId"],
	calendar: ["events"],
	gantt: ["tasks"],
};

/** Default BoundValue to inject when a required prop is missing. */
const DEFAULT_BOUND_VALUES: Record<string, BoundValue> = {
	content: { literalString: "" },
	src: { literalString: "" },
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
};

function defaultRequiredProp(type: string, prop: string): unknown {
	if (
		prop === "overlays" ||
		(type === "tabs" && prop === "tabs") ||
		(type === "accordion" && prop === "items")
	) {
		return [];
	}
	if (
		prop === "columns" ||
		prop === "data" ||
		prop === "cells" ||
		prop === "choices" ||
		prop === "items" ||
		prop === "boxes" ||
		prop === "labels" ||
		prop === "hotspots"
	) {
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
	validIds: Set<string>,
): { explicitList: string[] } | { template: unknown } | undefined {
	if (children == null || typeof children !== "object") return undefined;

	const c = children as Record<string, unknown>;

	if ("explicitList" in c && Array.isArray(c.explicitList)) {
		const filtered = (c.explicitList as unknown[]).filter(
			(id): id is string => typeof id === "string" && validIds.has(id),
		);
		return filtered.length > 0 ? { explicitList: filtered } : undefined;
	}

	if ("template" in c && c.template && typeof c.template === "object") {
		const template = c.template as Record<string, unknown>;
		if (
			typeof template.templateComponentId === "string" &&
			validIds.has(template.templateComponentId) &&
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

			actions.push({ name, context: { ...context } });
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

/**
 * Validate and sanitise an array of AI-generated SurfaceComponents.
 *
 * - Strips unknown component types
 * - Removes hallucinated props
 * - Coerces bare primitives → BoundValue
 * - Injects defaults for required props
 * - Prunes stale children references
 * - Ensures every component has an `id`
 */
export function validateComponents(
	raw: SurfaceComponent[],
	canvasSettings?: CanvasSettings,
): ValidationResult {
	const warnings: string[] = [];
	const registeredTypes = new Set(getRegisteredTypes());
	const input = raw.slice(0, MAX_COMPONENTS);
	if (raw.length > MAX_COMPONENTS) {
		warnings.push(`Only the first ${MAX_COMPONENTS} components were kept`);
	}

	// Build a set of all IDs for children validation
	const allIds = new Set(
		input
			.map((c) => c.id)
			.filter((id): id is string => typeof id === "string" && id.length > 0),
	);
	const seenIds = new Set<string>();

	const validated: SurfaceComponent[] = [];

	for (const comp of input) {
		// Must have an id
		if (!comp.id || typeof comp.id !== "string") {
			warnings.push("Skipped component with missing id");
			continue;
		}
		const componentId = comp.id.trim();
		if (componentId.length > MAX_COMPONENT_ID_CHARS) {
			warnings.push(`${componentId.slice(0, 32)}: component id is too long`);
			continue;
		}
		if (seenIds.has(componentId)) {
			warnings.push(`${componentId}: skipped duplicate component id`);
			continue;
		}
		seenIds.add(componentId);

		const rawComponent = comp.component as unknown as Record<string, unknown>;
		if (!rawComponent || typeof rawComponent !== "object") {
			warnings.push(`${componentId}: missing component data`);
			continue;
		}

		const type = rawComponent.type as string | undefined;
		if (!type || !registeredTypes.has(type)) {
			warnings.push(`${componentId}: unknown component type "${type}"`);
			continue;
		}

		// Build cleaned component object
		const knownForType = KNOWN_PROPS[type] ?? new Set<string>();
		const cleaned: Record<string, unknown> = { type };

		// Copy known props, coercing bare values to BoundValue
		for (const [key, value] of Object.entries(rawComponent)) {
			if (key === "type" || key === "id" || key === "children") continue; // handled separately
			if (BASE_PROPS.has(key)) {
				// Legacy actions pass through verbatim. Named handlers are additive but
				// structurally validated before they can suppress the legacy fallback.
				if (key === "eventHandlers") {
					const handlers = sanitizeEventHandlers(value, componentId, warnings);
					if (handlers !== undefined) cleaned[key] = handlers;
				} else {
					cleaned[key] = value;
				}
				continue;
			}

			if (knownForType.has(key)) {
				if (PLAIN_PROPS[type as A2UIComponentType]?.has(key)) {
					cleaned[key] = value;
					continue;
				}
				// Widget-instance props are raw wiring data (ids, the inline widget definition,
				// per-instance param/action values) — NOT BoundValue-wrapped component props, so
				// keep them verbatim instead of coercing.
				if (type === "widgetInstance") {
					cleaned[key] = value;
					continue;
				}
				if (type === "markdown" && key === "allowHtml") {
					cleaned[key] = { literalBool: false };
					warnings.push(`${componentId}: disabled HTML rendering for markdown`);
					continue;
				}
				if (type === "iframe" && key === "sandbox") {
					const sandbox = sanitizeIframeSandbox(value);
					if (sandbox) cleaned[key] = sandbox;
					continue;
				}
				if (type === "box" && key === "as") {
					const boxTag = coerceToBoundValue(value);
					if (boxTag && "path" in boxTag) {
						// Keep data binding semantics. A2UIBox applies the same allowlist
						// after resolving the path, including its default value.
						cleaned[key] = boxTag;
					} else if (
						boxTag &&
						"literalString" in boxTag &&
						isSemanticBoxTag(boxTag.literalString)
					) {
						cleaned[key] = boxTag;
					} else {
						const unsafeTag =
							boxTag && "literalString" in boxTag
								? ` "${boxTag.literalString}"`
								: "";
						cleaned[key] = {
							literalString: normalizeSemanticBoxTag(undefined),
						};
						warnings.push(
							`${componentId}: replaced unsafe box tag${unsafeTag} with "div"`,
						);
					}
					continue;
				}
				// Props that hold structured data (arrays/objects) should not be blindly coerced
				if (
					key === "tabs" ||
					key === "items" ||
					key === "overlays" ||
					key === "columns" ||
					key === "data" ||
					key === "boxes" ||
					key === "hotspots" ||
					key === "markers" ||
					key === "choices" ||
					key === "options"
				) {
					// Options specifically expects BoundValue
					if (key === "options") {
						cleaned[key] = coerceToBoundValue(value) ?? value;
					} else {
						cleaned[key] = value;
					}
				} else {
					cleaned[key] = coerceToBoundValue(value) ?? value;
				}
			} else {
				warnings.push(
					`${componentId}: stripped unknown prop "${key}" on ${type}`,
				);
			}
		}

		// Inject defaults for missing required props
		const required = REQUIRED_PROPS[type];
		let missingUnfixableRequiredProp = false;
		if (required) {
			for (const prop of required) {
				if (!(prop in cleaned)) {
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
			}
		}
		if (missingUnfixableRequiredProp) continue;

		// Validate children references
		const rawChildren = rawComponent.children;
		const validatedChildren = validateChildren(rawChildren, allIds);
		if (validatedChildren) {
			(cleaned as Record<string, unknown>).children = validatedChildren;
		} else if (rawChildren != null) {
			warnings.push(`${componentId}: removed invalid children reference`);
		}

		validated.push(
			normalizeSurfaceComponentForPersistence({
				id: componentId,
				style: comp.style,
				component: cleaned as unknown as SurfaceComponent["component"],
			}),
		);
	}

	const validIds = new Set(validated.map((component) => component.id));
	for (const comp of validated) {
		const component = comp.component as unknown as Record<string, unknown>;
		const validatedChildren = validateChildren(component.children, validIds);
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
 * Validate canvas settings from AI output.
 * Strips unknown keys and ensures values are sensible.
 */
export function validateCanvasSettings(
	raw: unknown,
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
		// CSS cannot be truncated safely at an arbitrary character boundary: a
		// cut inside a declaration, string, or nested at-rule invalidates the
		// stylesheet. Large stylesheets are parsed off-thread by ScopedCustomCss.
		result.customCss = obj.customCss;
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
