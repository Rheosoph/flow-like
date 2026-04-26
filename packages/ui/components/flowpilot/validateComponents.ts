/**
 * Validates and sanitises AI-generated SurfaceComponent arrays.
 *
 * LLMs may hallucinate props, omit required fields, use bare strings
 * instead of BoundValue wrappers, or reference unknown component types.
 * This module fixes what it can and strips the rest so the A2UI renderer
 * never receives invalid data.
 */

import { getRegisteredTypes } from "../a2ui/ComponentRegistry";
import type {
	BoundValue,
	CanvasSettings,
	SurfaceComponent,
} from "../a2ui/types";

// ---------------------------------------------------------------------------
// Known props per component type
// ---------------------------------------------------------------------------

/** Which props a given component type accepts (excluding the shared base:
 *  `type`, `id`, `style`, `children`, `actions`). */
const KNOWN_PROPS: Record<string, Set<string>> = {
	// Layout
	row: new Set(["gap", "align", "justify", "wrap", "reverse"]),
	column: new Set(["gap", "align", "justify", "reverse", "wrap"]),
	stack: new Set(["align", "width", "height"]),
	grid: new Set(["columns", "rows", "gap", "columnGap", "rowGap", "autoFlow"]),
	scrollArea: new Set(["direction"]),
	aspectRatio: new Set(["ratio"]),
	overlay: new Set(["baseComponentId", "overlays"]),
	absolute: new Set(["width", "height"]),
	box: new Set(["as", "semanticRole"]),
	center: new Set(["inline"]),
	spacer: new Set(["size", "flex", "direction", "flexible"]),
	widgetInstance: new Set(["widgetId", "widgetInputs", "bindOutputs"]),

	// Display
	text: new Set([
		"content",
		"variant",
		"size",
		"weight",
		"color",
		"align",
		"truncate",
		"maxLines",
	]),
	image: new Set([
		"src",
		"alt",
		"fit",
		"fallback",
		"fallbackSrc",
		"loading",
		"aspectRatio",
		"width",
		"height",
	]),
	icon: new Set(["name", "size", "color", "strokeWidth"]),
	video: new Set([
		"src",
		"poster",
		"autoplay",
		"autoPlay",
		"loop",
		"muted",
		"controls",
		"width",
		"height",
	]),
	lottie: new Set(["src", "autoplay", "loop", "speed", "width", "height"]),
	markdown: new Set(["content", "allowHtml"]),
	divider: new Set(["orientation", "thickness", "color"]),
	badge: new Set(["content", "text", "variant", "color"]),
	avatar: new Set(["src", "fallback", "size"]),
	progress: new Set(["value", "max", "showLabel", "variant", "color"]),
	spinner: new Set(["size", "color"]),
	skeleton: new Set(["width", "height", "rounded", "variant"]),
	iframe: new Set([
		"src",
		"srcdoc",
		"width",
		"height",
		"sandbox",
		"allow",
		"title",
		"referrerPolicy",
		"border",
		"loading",
	]),
	table: new Set([
		"columns",
		"data",
		"caption",
		"striped",
		"bordered",
		"hoverable",
		"compact",
		"stickyHeader",
		"sortable",
		"searchable",
		"paginated",
		"pageSize",
		"selectable",
		"onRowClick",
		"showPagination",
	]),
	tableRow: new Set(["cells", "selected", "disabled"]),
	tableCell: new Set(["content", "isHeader", "colSpan", "rowSpan", "align"]),
	plotlyChart: new Set([
		"chartType",
		"data",
		"title",
		"layout",
		"config",
		"height",
		"width",
	]),
	nivoChart: new Set([
		"chartType",
		"data",
		"height",
		"width",
		"colors",
		"colorScheme",
		"showLegend",
		"legendPosition",
		"margin",
		"axisBottom",
		"axisLeft",
		"animate",
		"motionConfig",
		"style",
	]),
	filePreview: new Set(["url", "mimeType", "width", "height"]),
	boundingBoxOverlay: new Set([
		"src",
		"boxes",
		"showLabels",
		"showConfidence",
		"normalized",
		"width",
		"height",
	]),
	geoMap: new Set([
		"center",
		"zoom",
		"markers",
		"style",
		"width",
		"height",
		"mapStyle",
		"interactive",
	]),

	// Interactive
	button: new Set([
		"label",
		"variant",
		"size",
		"disabled",
		"loading",
		"icon",
		"iconPosition",
		"tooltip",
	]),
	textField: new Set([
		"value",
		"placeholder",
		"label",
		"helperText",
		"error",
		"disabled",
		"inputType",
		"type",
		"multiline",
		"rows",
		"maxLength",
		"required",
	]),
	select: new Set([
		"value",
		"options",
		"placeholder",
		"label",
		"disabled",
		"multiple",
		"searchable",
	]),
	slider: new Set([
		"value",
		"min",
		"max",
		"step",
		"disabled",
		"showValue",
		"label",
	]),
	checkbox: new Set(["checked", "label", "disabled", "indeterminate"]),
	switch: new Set(["checked", "label", "disabled"]),
	radioGroup: new Set(["value", "options", "disabled", "orientation", "label"]),
	dateTimeInput: new Set(["value", "mode", "min", "max", "disabled", "label"]),
	fileInput: new Set([
		"value",
		"label",
		"helperText",
		"accept",
		"multiple",
		"maxSize",
		"maxFiles",
		"disabled",
		"error",
	]),
	imageInput: new Set([
		"value",
		"label",
		"helperText",
		"accept",
		"multiple",
		"maxSize",
		"maxFiles",
		"disabled",
		"error",
		"aspectRatio",
		"showPreview",
	]),
	imageLabeler: new Set([
		"src",
		"labels",
		"boxes",
		"disabled",
		"width",
		"height",
	]),
	imageHotspot: new Set(["src", "hotspots", "markerStyle", "width", "height"]),
	link: new Set([
		"href",
		"label",
		"text",
		"route",
		"queryParams",
		"external",
		"target",
		"variant",
		"underline",
		"disabled",
		"openInNewTab",
	]),

	// Container
	card: new Set([
		"title",
		"description",
		"footer",
		"hoverable",
		"clickable",
		"variant",
		"padding",
		"headerImage",
		"headerIcon",
	]),
	modal: new Set([
		"open",
		"title",
		"description",
		"closeOnOverlay",
		"closeOnEscape",
		"showCloseButton",
		"size",
		"centered",
	]),
	tabs: new Set(["value", "tabs", "orientation", "variant", "defaultValue"]),
	accordion: new Set([
		"items",
		"multiple",
		"defaultExpanded",
		"collapsible",
		"type",
	]),
	drawer: new Set([
		"open",
		"side",
		"title",
		"size",
		"overlay",
		"closable",
		"description",
	]),
	tooltip: new Set(["content", "side", "delayMs", "maxWidth"]),
	popover: new Set([
		"open",
		"contentComponentId",
		"side",
		"trigger",
		"closeOnClickOutside",
		"content",
	]),

	// Game
	canvas2d: new Set(["width", "height", "backgroundColor", "pixelPerfect"]),
	sprite: new Set([
		"src",
		"x",
		"y",
		"width",
		"height",
		"rotation",
		"scale",
		"opacity",
		"flipX",
		"flipY",
		"zIndex",
	]),
	shape: new Set([
		"shapeType",
		"x",
		"y",
		"width",
		"height",
		"radius",
		"points",
		"fill",
		"stroke",
		"strokeWidth",
	]),
	scene3d: new Set([
		"width",
		"height",
		"cameraType",
		"cameraPosition",
		"backgroundColor",
		"controlMode",
		"fixedView",
		"autoRotateSpeed",
		"enableControls",
		"enableZoom",
		"enablePan",
		"fov",
		"near",
		"far",
		"target",
		"ambientLight",
		"directionalLight",
		"showGrid",
		"showAxes",
	]),
	model3d: new Set([
		"src",
		"position",
		"rotation",
		"scale",
		"castShadow",
		"receiveShadow",
		"animation",
		"autoRotate",
		"rotateSpeed",
		"viewerHeight",
		"backgroundColor",
		"cameraDistance",
		"fov",
		"cameraAngle",
		"cameraPosition",
		"cameraTarget",
		"enableControls",
		"enableZoom",
		"enablePan",
		"autoRotateCamera",
		"cameraRotateSpeed",
		"ambientLight",
		"directionalLight",
		"fillLight",
		"rimLight",
		"lightColor",
		"lightingPreset",
		"showGround",
		"groundColor",
		"enableReflections",
		"environment",
		"environmentSource",
		"useHdrBackground",
		"polyhavenHdri",
		"polyhavenResolution",
	]),
	dialogue: new Set([
		"text",
		"speakerName",
		"typewriter",
		"speed",
		"portrait",
		"children",
	]),
	characterPortrait: new Set([
		"image",
		"expression",
		"position",
		"width",
		"height",
		"flip",
	]),
	choiceMenu: new Set(["choices", "title", "layout", "columns"]),
	inventoryGrid: new Set([
		"items",
		"columns",
		"rows",
		"cellSize",
		"showTooltips",
	]),
	healthBar: new Set([
		"value",
		"maxValue",
		"label",
		"fillColor",
		"variant",
		"showLabel",
		"size",
		"animated",
	]),
	miniMap: new Set([
		"mapImage",
		"width",
		"height",
		"markers",
		"playerX",
		"playerY",
		"viewportWidth",
		"viewportHeight",
		"zoom",
	]),
};

/** Shared base props all component types have. */
const BASE_PROPS = new Set(["type", "id", "style", "children", "actions"]);

/** Required props that MUST exist (non-optional in the TS interface). */
const REQUIRED_PROPS: Record<string, string[]> = {
	text: ["content"],
	image: ["src"],
	icon: ["name"],
	video: ["src"],
	lottie: ["src"],
	markdown: ["content"],
	badge: ["content"],
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
	link: ["href"],
	modal: ["open"],
	tabs: ["value"],
	canvas2d: ["width", "height"],
	sprite: ["src", "x", "y"],
	shape: ["shapeType", "x", "y"],
	scene3d: ["width", "height"],
	model3d: ["src"],
	aspectRatio: ["ratio"],
	boundingBoxOverlay: ["src"],
};

/** Default BoundValue to inject when a required prop is missing. */
const DEFAULT_BOUND_VALUES: Record<string, BoundValue> = {
	content: { literalString: "" },
	src: { literalString: "" },
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
};

const MAX_COMPONENTS = 120;
const MAX_COMPONENT_ID_CHARS = 120;
const MAX_BOUND_STRING_CHARS = 8_000;
const MAX_CUSTOM_CSS_CHARS = 12_000;

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
			if (key === "type") continue;
			if (BASE_PROPS.has(key)) continue; // handled separately

			if (knownForType.has(key)) {
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
		if (required) {
			for (const prop of required) {
				if (!(prop in cleaned)) {
					const defaultVal = DEFAULT_BOUND_VALUES[prop];
					if (defaultVal) {
						cleaned[prop] = defaultVal;
						warnings.push(
							`${componentId}: injected default for required prop "${prop}" on ${type}`,
						);
					}
				}
			}
		}

		// Validate children references
		const rawChildren = rawComponent.children;
		const validatedChildren = validateChildren(rawChildren, allIds);
		if (validatedChildren) {
			(cleaned as Record<string, unknown>).children = validatedChildren;
		} else if (rawChildren != null) {
			warnings.push(`${componentId}: removed invalid children reference`);
		}

		validated.push({
			id: componentId,
			style: comp.style,
			component: cleaned as unknown as SurfaceComponent["component"],
		});
	}

	const validIds = new Set(validated.map((component) => component.id));
	for (const comp of validated) {
		const component = comp.component as unknown as Record<string, unknown>;
		const validatedChildren = validateChildren(component.children, validIds);
		if (validatedChildren) {
			component.children = validatedChildren;
		} else if (component.children != null) {
			delete component.children;
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
		result.customCss = obj.customCss.slice(0, MAX_CUSTOM_CSS_CHARS);
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
