import type { IChannelHandle } from "../schema/channel";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import type { ILogMetadata } from "../schema/flow/log-metadata";
import type { ExecutionChatSeed } from "./chat-persistence";

/** Enumerable symbols survive payload spreads without entering the HTTP JSON body. */
export const EXECUTION_METADATA: unique symbol = Symbol.for(
	"flow-like.execution.metadata",
);

export interface ExecutionMetadata {
	taskId: string;
	streamId: string;
	scope: string;
	appId: string;
	eventId: string;
	title: string;
	path?: string;
	interfaceType?: string;
	pageId?: string;
	widgetInstanceId?: string;
	startedAt: string;
	chat?: ExecutionChatSeed;
}

export function getExecutionMetadata(
	payload: unknown,
): ExecutionMetadata | undefined {
	return payload && typeof payload === "object"
		? (payload as { [EXECUTION_METADATA]?: ExecutionMetadata })[
				EXECUTION_METADATA
			]
		: undefined;
}

export interface PreparedExecutionRequest {
	url: string;
	headers?: Record<string, string>;
	body: unknown;
	/** Replace {runId} after the server announces the execution. */
	cancelUrl?: string;
	/** Bounds dispatch until response headers, never the lifetime of the SSE stream. */
	timeoutMs?: number;
	credentials?: RequestCredentials;
}

export type ExecutionTaskStatus =
	| "starting"
	| "running"
	| "waiting"
	| "completed"
	| "failed"
	| "cancelled";

export interface ExecutionInteraction {
	requestId: string;
	event: IIntercomEvent;
	channel?: IChannelHandle;
	status: "pending" | "submitting" | "submitted" | "expired" | "cancelled";
	error?: string;
	response_value?: unknown;
}

export interface ExecutionTask extends ExecutionMetadata {
	status: ExecutionTaskStatus;
	runId?: string;
	completedAt?: number;
	error?: string;
	preview?: string;
	events: IIntercomEvent[];
	pendingInteractions: ExecutionInteraction[];
	result?: ILogMetadata;
	/** Only remote tasks whose events and chat writes belong to this runtime. */
	persistenceManaged?: boolean;
	cancellationRequested?: boolean;
}

export function isExecutionFinished(
	task: Pick<ExecutionTask, "status">,
): boolean {
	return (
		task.status === "completed" ||
		task.status === "failed" ||
		task.status === "cancelled"
	);
}

export interface ExecutionCallbacks {
	onEvents?: (events: IIntercomEvent[]) => void;
	onExecutionStart?: (runId: string) => void;
	onResponse?: () => void;
	onUnauthorized?: (
		rejectedToken?: string,
	) => Promise<Record<string, string> | undefined>;
}

export interface ExecutionCancelOptions {
	headers?: Record<string, string>;
	onUnauthorized?: ExecutionCallbacks["onUnauthorized"];
}

export interface ExecutionErrorData {
	name: string;
	message: string;
	status?: number;
	body?: string;
	url?: string;
	statusText?: string;
	headers?: Record<string, string>;
	timeoutMs?: number;
}

export function executionError(error: unknown): ExecutionErrorData {
	const source = error as Partial<ExecutionErrorData> | null;
	return {
		name: typeof source?.name === "string" ? source.name : "Error",
		message:
			typeof source?.message === "string" ? source.message : String(error),
		...(typeof source?.status === "number" ? { status: source.status } : {}),
		...(typeof source?.body === "string" ? { body: source.body } : {}),
		...(typeof source?.url === "string" ? { url: source.url } : {}),
		...(typeof source?.statusText === "string"
			? { statusText: source.statusText }
			: {}),
		...(source?.headers ? { headers: source.headers } : {}),
		...(typeof source?.timeoutMs === "number"
			? { timeoutMs: source.timeoutMs }
			: {}),
	};
}

export type ExecutionCommand =
	| { type: "connect"; id: string; scope: string }
	| { type: "disconnect"; id: string }
	| {
			type: "execute";
			id: string;
			metadata: ExecutionMetadata;
			request: PreparedExecutionRequest;
	  }
	| {
			type: "cancel";
			id: string;
			taskId: string;
			headers?: Record<string, string>;
	  }
	| { type: "dismiss"; id: string; taskId: string }
	| { type: "reply"; id: string; handle: IChannelHandle; value: unknown }
	| { type: "claim"; id: string; handle: IChannelHandle }
	| { type: "claim-event"; id: string; taskId: string; eventId: string };

export interface ExecutionAuthorization {
	type: "authorize";
	id: string;
	headers?: Record<string, string>;
	error?: ExecutionErrorData;
}

export type ExecutionMessage =
	| { type: "snapshot"; tasks: ExecutionTask[] }
	| { type: "response"; taskId: string }
	| { type: "update"; task: ExecutionTask; offset: number }
	| { type: "unauthorized"; id: string; taskId: string; rejectedToken?: string }
	| { type: "result"; id: string; value?: unknown; error?: ExecutionErrorData };

export interface ExecutionPort {
	postMessage(message: ExecutionMessage): void;
}
