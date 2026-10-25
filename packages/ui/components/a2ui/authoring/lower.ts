import {
	type IndexedComponentSpec,
	type PropSpec,
	componentSpec,
} from "./manifest";

/**
 * Plain-value authoring form → canonical `SurfaceComponent` JSON. A port of
 * `lower_component` in `packages/core/a2ui-schema/src/authoring.rs`; both read
 * the same generated manifest, so the frontend and the backend accept the same
 * input. Inputs are never mutated.
 */

type JsonObject = Record<string, unknown>;

/** A lowered component: surface keys at the top, everything else in `component`. */
export type LoweredComponent = JsonObject & { component: JsonObject };

export type LowerOutcome =
	| { readonly ok: true; readonly component: LoweredComponent }
	| { readonly ok: false; readonly error: string };

export interface AuthoringError {
	readonly index: number;
	readonly id?: string;
	readonly message: string;
}

const BOUND_VALUE_KEYS = [
	"literalString",
	"literalNumber",
	"literalBool",
	"literalJson",
	"literalOptions",
	"path",
] as const;
const SURFACE_KEYS: ReadonlySet<string> = new Set([
	"id",
	"style",
	"eventRelevant",
]);
const VERBATIM_KEYS: ReadonlySet<string> = new Set(["type", "id", "style"]);
const MAX_FIELD_DEPTH = 3;
const MAX_INLINE_WIDGET_DEPTH = 4;

export function isJsonObject(value: unknown): value is JsonObject {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Lower one authored component (flat or nested, plain or wrapped values),
 * including the trees of `widgetInstance.inlineWidgetDef`. Unknown types and
 * props are kept for the validators to judge.
 */
export function lowerAuthoredComponent(value: unknown) {
	return lowerComponentAt(value, 0);
}

function lowerComponentAt(value: unknown, depth: number): LowerOutcome {
	const parts = authoredParts(value);
	if (typeof parts === "string") return { ok: false, error: parts };
	const component = lowerComponentProps(parts.type, parts.component);
	const error = lowerInlineWidget(component, depth);
	if (error) return { ok: false, error };
	return {
		ok: true,
		component: { ...parts.surface, component } as LoweredComponent,
	};
}

/** The surface keys, props and type of an authored component, or why it is not one. */
function authoredParts(value: unknown) {
	if (!isJsonObject(value)) return "a component must be a JSON object";
	const split = splitAuthored(value);
	if (!split) return "`component` must be an object with a `type`";
	const type = split.component.type;
	return typeof type === "string"
		? { ...split, type }
		: "missing component `type`";
}

/** Lower `inlineWidgetDef.components` in the lowered props; a failing child fails the whole. */
function lowerInlineWidget(component: JsonObject, depth: number) {
	if (depth >= MAX_INLINE_WIDGET_DEPTH) return undefined;
	const definition = component.inlineWidgetDef;
	if (!isJsonObject(definition) || !Array.isArray(definition.components)) {
		return undefined;
	}
	const children: LoweredComponent[] = [];
	for (const [index, child] of definition.components.entries()) {
		const outcome = lowerComponentAt(child, depth + 1);
		if (!outcome.ok) {
			return `inlineWidgetDef.components[${index}]: ${outcome.error}`;
		}
		children.push(outcome.component);
	}
	component.inlineWidgetDef = { ...definition, components: children };
	return undefined;
}

/** Separate surface keys from component props; undefined when `component` is not an object. */
function splitAuthored(value: JsonObject) {
	if (Object.hasOwn(value, "component")) {
		const { component, ...surface } = value;
		return isJsonObject(component) ? { surface, component } : undefined;
	}
	const surface: JsonObject = {};
	const component: JsonObject = {};
	for (const [key, entry] of Object.entries(value)) {
		if (SURFACE_KEYS.has(key)) surface[key] = entry;
		else component[key] = entry;
	}
	return { surface, component };
}

/** Lower every component, collecting errors for the ones that cannot be lowered. */
export function lowerAuthoredComponents(values: readonly unknown[]): {
	components: LoweredComponent[];
	errors: AuthoringError[];
} {
	const components: LoweredComponent[] = [];
	const errors: AuthoringError[] = [];
	values.forEach((value, index) => {
		const outcome = lowerAuthoredComponent(value);
		if (outcome.ok) {
			components.push(outcome.component);
			return;
		}
		const id =
			isJsonObject(value) && typeof value.id === "string"
				? value.id
				: undefined;
		errors.push({ index, id, message: outcome.error });
	});
	return { components, errors };
}

function lowerComponentProps(type: string, component: JsonObject) {
	const spec = componentSpec(type);
	const lowered: JsonObject = {};
	for (const [key, value] of Object.entries(component)) {
		if (key === "action") continue;
		const next = lowerProp(spec, key, value);
		if (next !== undefined) lowered[key] = next;
	}
	if (Object.hasOwn(component, "action")) {
		appendAction(lowered, component.action);
	}
	return lowered;
}

function lowerProp(
	spec: IndexedComponentSpec | undefined,
	key: string,
	value: unknown,
) {
	if (VERBATIM_KEYS.has(key)) return value;
	if (key === "actions") return lowerActions(value);
	if (key === "eventHandlers") return lowerEventHandlers(value);
	if (key === "children") return lowerChildren(value);
	if (key === "hidden") return lowerBound("hidden", value);
	const prop = spec?.props.get(key);
	return prop ? lowerBySpec(prop, value, 0) : value;
}

/** The `action` shorthand is normalized like `actions` and appended to it. */
function appendAction(lowered: JsonObject, action: unknown) {
	if (!Object.hasOwn(lowered, "actions")) lowered.actions = [];
	const actions = lowered.actions;
	if (Array.isArray(actions)) {
		lowered.actions = [...actions, ...lowerActions(action)];
	}
}

/**
 * Normalize an action list: a single action becomes a one-item list, a bare name
 * becomes `{"name": …}` and every action carries a `context`. Entries that are not
 * actions drop.
 */
export function lowerActions(value: unknown): JsonObject[] {
	const items = Array.isArray(value) ? value : [value];
	return items
		.map(lowerAction)
		.filter((action): action is JsonObject => action !== undefined);
}

function lowerAction(value: unknown): JsonObject | undefined {
	if (typeof value === "string") {
		return value.trim() ? { name: value, context: {} } : undefined;
	}
	if (!isJsonObject(value) || typeof value.name !== "string") return undefined;
	return Object.hasOwn(value, "context") ? value : { ...value, context: {} };
}

function lowerEventHandlers(value: unknown) {
	if (!isJsonObject(value)) return value;
	return Object.fromEntries(
		Object.entries(value).map(([event, actions]) => [
			event,
			lowerActions(actions),
		]),
	);
}

function lowerBySpec(spec: PropSpec, value: unknown, depth: number) {
	switch (spec.kind) {
		case "bound":
			return lowerBound(spec.name, value);
		case "plain":
			return unwrapLiteral(value);
		case "list":
			return Array.isArray(value)
				? value.map((item) => lowerFields(spec.fields ?? [], item, depth + 1))
				: value;
		case "object":
			return lowerFields(spec.fields ?? [], value, depth + 1);
		default:
			return value;
	}
}

function lowerFields(
	fields: readonly PropSpec[],
	value: unknown,
	depth: number,
): unknown {
	if (!isJsonObject(value)) return value;
	if (depth > MAX_FIELD_DEPTH || fields.length === 0) return value;
	const lowered: JsonObject = { ...value };
	for (const field of fields) {
		if (!Object.hasOwn(lowered, field.name)) continue;
		const raw = lowered[field.name];
		delete lowered[field.name];
		const next = lowerBySpec(field, raw, depth);
		if (next !== undefined) lowered[field.name] = next;
	}
	return lowered;
}

/**
 * Wrap a plain JSON value as a `BoundValue`. Existing bound values pass through;
 * `null` returns `undefined`, which removes the prop.
 */
export function lowerBound(propName: string, value: unknown): unknown {
	if (value === null || value === undefined) return undefined;
	if (typeof value === "string") return { literalString: value };
	if (typeof value === "number") return { literalNumber: value };
	if (typeof value === "boolean") return { literalBool: value };
	if (isJsonObject(value) && isBoundValue(value)) return value;
	if (Array.isArray(value) && propName === "options") {
		const options = selectOptions(value);
		return options
			? { literalOptions: options }
			: { literalJson: JSON.stringify(value) };
	}
	return { literalJson: JSON.stringify(value) };
}

/**
 * An id array (or a single id) becomes `{"explicitList": [...]}`: numeric ids become
 * strings, anything else in the list drops. `null` removes children.
 */
export function lowerChildren(value: unknown): unknown {
	if (value === null || value === undefined) return undefined;
	if (typeof value === "string") return { explicitList: [value] };
	if (Array.isArray(value)) {
		return {
			explicitList: value
				.filter((id) => typeof id === "string" || typeof id === "number")
				.map(String),
		};
	}
	return value;
}

function isBoundValue(object: JsonObject): boolean {
	return BOUND_VALUE_KEYS.some((key) => Object.hasOwn(object, key));
}

function unwrapLiteral(value: unknown): unknown {
	if (!isJsonObject(value)) return value;
	const keys = Object.keys(value);
	if (keys.length !== 1) return value;
	const [key] = keys;
	return key.startsWith("literal") &&
		key !== "literalJson" &&
		key !== "literalOptions"
		? value[key]
		: value;
}

function selectOptions(
	items: readonly unknown[],
): { value: string; label: string }[] | undefined {
	const options: { value: string; label: string }[] = [];
	for (const item of items) {
		if (typeof item === "string") {
			options.push({ value: item, label: item });
		} else if (typeof item === "number") {
			options.push({ value: String(item), label: String(item) });
		} else if (isJsonObject(item)) {
			const value = scalarText(item.value);
			if (value === undefined) return undefined;
			options.push({ value, label: scalarText(item.label) ?? value });
		} else {
			return undefined;
		}
	}
	return options;
}

function scalarText(value: unknown): string | undefined {
	if (typeof value === "string") return value;
	if (typeof value === "number" || typeof value === "boolean") {
		return String(value);
	}
	return undefined;
}
