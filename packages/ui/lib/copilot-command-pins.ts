import { IPinType } from "./schema/flow/pin";

/**
 * Function-layer boundary pins face the enclosing graph: an Input supplies a parameter to the
 * body and is therefore the source of an inner edge; an Output receives a body return and is the
 * target. Ordinary nodes keep their declared direction.
 */
export function expectedCopilotPinType(
	expected: IPinType | undefined,
	isLayerBoundary: boolean,
): IPinType | undefined {
	if (!isLayerBoundary || expected === undefined) return expected;
	return expected === IPinType.Input ? IPinType.Output : IPinType.Input;
}
