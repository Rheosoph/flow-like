import type { IScopedFlowScriptResponse } from "../../state/backend-state/board-state";
import { asArray, isRecord } from "../response-shape";

/** The `flowscript` text of a hub response; an empty or garbled body throws instead of rendering `undefined`. */
export function flowScriptFromResponse(
	response: unknown,
	boardId: string,
): string {
	const flowscript = isRecord(response) ? response.flowscript : undefined;
	if (typeof flowscript !== "string") {
		throw new Error(`The hub returned no FlowScript for board ${boardId}`);
	}
	return flowscript;
}

export function scopedFlowScriptFromResponse(
	response: unknown,
	boardId: string,
): IScopedFlowScriptResponse {
	const flowscript = flowScriptFromResponse(response, boardId);
	const anchors = isRecord(response) ? response.scope_anchors : undefined;
	return {
		flowscript,
		scope_anchors: asArray(anchors as string[] | undefined),
	};
}
