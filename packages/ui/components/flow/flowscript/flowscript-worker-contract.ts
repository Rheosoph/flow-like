import type { INode } from "../../../lib/schema/flow/node";
import type {
	FlowScriptDocumentSymbol,
	FlowScriptEnvContext,
	FlowScriptFoldingRange,
	FlowScriptInlayHint,
} from "./flowscript-language-features";

export interface FlowScriptWorkerModelLike {
	uri: unknown;
	getValue: () => string;
	getVersionId?: () => number;
}

export interface CancellationTokenLike {
	isCancellationRequested?: boolean;
	onCancellationRequested?: (
		listener: () => void,
	) => { dispose: () => void } | undefined;
}

export type FlowScriptWorkerOutcome<T> =
	| { status: "ok"; value: T }
	| { status: "cancelled" }
	| { status: "failed" };

/** Worker-backed requests used by Monaco's FlowScript language providers. */
export interface FlowScriptWorkerRequests {
	requestEnvDoc: (
		model: FlowScriptWorkerModelLike,
		nodes: INode[] | undefined,
		token?: CancellationTokenLike,
	) => Promise<FlowScriptWorkerOutcome<FlowScriptEnvContext>> | null;
	requestDocumentSymbols: (
		model: FlowScriptWorkerModelLike,
		nodes: INode[] | undefined,
		token?: CancellationTokenLike,
	) => Promise<FlowScriptWorkerOutcome<FlowScriptDocumentSymbol[]>> | null;
	requestFolding: (
		model: FlowScriptWorkerModelLike,
		nodes: INode[] | undefined,
		token?: CancellationTokenLike,
	) => Promise<FlowScriptWorkerOutcome<FlowScriptFoldingRange[]>> | null;
	requestInlayHints: (
		model: FlowScriptWorkerModelLike,
		nodes: INode[] | undefined,
		startLine: number,
		endLine: number,
		token?: CancellationTokenLike,
	) => Promise<FlowScriptWorkerOutcome<FlowScriptInlayHint[]>> | null;
	requestSemanticTokens: (
		model: FlowScriptWorkerModelLike,
		nodes: INode[] | undefined,
		token?: CancellationTokenLike,
	) => Promise<FlowScriptWorkerOutcome<Uint32Array>> | null;
}
