import { IExecutionMode } from "../../lib/schema/flow/board";

export interface FlowRunCapabilityInputs {
	executionMode?: IExecutionMode;
	/** Offline apps can never reach the server; hosts without remote wiring pass `hasRemoteExecute: false`. */
	isOffline?: boolean;
	hasRemoteExecute: boolean;
	onlyOffline?: boolean | null;
}

export interface FlowRunCapabilities {
	canLocalExecute: boolean;
	canRemoteExecute: boolean;
}

/**
 * The single derivation for "can this start node run locally / on the server".
 * Shared by the canvas play button (flow-node) and the FlowScript run lenses so
 * the two surfaces can never disagree.
 */
export function deriveRunCapabilities({
	executionMode,
	isOffline,
	hasRemoteExecute,
	onlyOffline,
}: FlowRunCapabilityInputs): FlowRunCapabilities {
	const mode = executionMode ?? IExecutionMode.Hybrid;
	return {
		canLocalExecute: mode !== IExecutionMode.Remote,
		canRemoteExecute:
			!isOffline &&
			hasRemoteExecute &&
			mode !== IExecutionMode.Local &&
			!onlyOffline,
	};
}
