import type { SurfaceComponent } from "./types";
import { elementValueTargets } from "./workflow-elements";

/**
 * Components whose renderer stores the user's edit under its element id: a `change`
 * action carrying `value` or `checked`. Only these can hold an edit a flow write replaces.
 */
const VALUE_INPUT_TYPES: ReadonlySet<string> = new Set([
	"checkbox",
	"dateTimeInput",
	"excalidraw",
	"fileInput",
	"imageInput",
	"imageLabeler",
	"radioGroup",
	"richText",
	"select",
	"slider",
	"switch",
	"tabs",
	"textField",
	"voiceInput",
]);

interface BoundInput {
	value: unknown;
	revision: number | undefined;
}

export interface FlowValueBaseline {
	scope: string;
	inputs: ReadonlyMap<string, BoundInput>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

const LITERAL_KEYS = [
	"literalString",
	"literalNumber",
	"literalBool",
	"literalOptions",
] as const;

/** The value a literal binding shows; undefined for a path, which lives in the data context. */
function literalValue(bound: unknown) {
	if (!isRecord(bound)) return bound;
	const key = LITERAL_KEYS.find((candidate) => candidate in bound);
	if (key) return bound[key];
	if (typeof bound.literalJson !== "string") return undefined;
	try {
		return JSON.parse(bound.literalJson) as unknown;
	} catch {
		return undefined;
	}
}

function boundInput(data: Record<string, unknown>) {
	if (typeof data.type !== "string" || !VALUE_INPUT_TYPES.has(data.type)) {
		return undefined;
	}
	const revision = data.valueRevision;
	const input: BoundInput = {
		value: data.value,
		revision: typeof revision === "number" ? revision : undefined,
	};
	return input;
}

function sameContent(left: unknown, right: unknown) {
	return left === right || JSON.stringify(left) === JSON.stringify(right);
}

function rewritten(previous: BoundInput | undefined, next: BoundInput) {
	if (!previous) return next.revision !== undefined;
	return (
		previous.revision !== next.revision ||
		!sameContent(previous.value, next.value)
	);
}

/**
 * The input values a flow replaced since `previous`, keyed like the user's edits of them.
 *
 * An input gives up what the user typed once its bound value moves or its `valueRevision`
 * advances (see `useBoundInputValue`). The stored edit has to go with it, or every run
 * payload keeps carrying a value the screen no longer shows. The first pass over a scope
 * only records a baseline: values restored for it are the user's, not stale.
 */
export function collectFlowValueWrites(
	previous: FlowValueBaseline | null,
	scope: string,
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
): { baseline: FlowValueBaseline; writes: Array<[string, unknown]> } {
	const inputs = new Map<string, BoundInput>();
	const writes: Array<[string, unknown]> = [];
	const known = previous?.scope === scope ? previous.inputs : undefined;

	for (const [elementId, data] of elementValueTargets(components, surfaceId)) {
		const input = boundInput(data);
		if (!input) continue;
		inputs.set(elementId, input);
		if (!known || !rewritten(known.get(elementId), input)) continue;

		const value = literalValue(input.value);
		if (value !== undefined) writes.push([elementId, value]);
	}

	return { baseline: { scope, inputs }, writes };
}

/**
 * The restored values among `restoredKeys` that their input does not show.
 *
 * Nothing pushes a restored value into its input, which goes on rendering its declared
 * value. Sending the restored one would hand a workflow a draft the user cannot see. A path
 * binding renders the data context, so a restored value never matches what it shows.
 */
export function hiddenRestoredInputs(
	values: Readonly<Record<string, unknown>>,
	restoredKeys: ReadonlySet<string>,
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
): string[] {
	const hidden: string[] = [];
	for (const [elementId, data] of elementValueTargets(components, surfaceId)) {
		if (!restoredKeys.has(elementId)) continue;
		const input = boundInput(data);
		if (!input) continue;
		const shown = literalValue(input.value);
		if (shown === undefined || !sameContent(values[elementId], shown)) {
			hidden.push(elementId);
		}
	}
	return hidden;
}
