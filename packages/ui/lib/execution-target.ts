/*
 * Where a run of an event executes, decided before it is sent: the upload of a file for that run
 * must land where the run reads it (a FlowPath in the server's temporary store for the cloud, in
 * this device's temporary store for a local run), and the cloud has a per-account cap. The rule is
 * the execution service's (state/execution-service-provider.tsx, `checkAndExecuteEvent`), so the
 * form uploads where the run will go. A2UI's `resolveTemporaryUploadTarget` keeps its own copy of
 * the board variant until its file can adopt this one.
 */
import type { IEventState } from "../state/backend-state/event-state";
import type { ITemporaryUploadExecutionTarget } from "../state/backend-state/helper-state";
import { prerunEventKey, prerunSwr } from "../state/backend-state/prerun-cache";
import { isRecord } from "./response-shape";
import { IExecutionMode } from "./schema/flow/board";
import { IEventExecutionMode } from "./schema/flow/event";

/** What a prerun answer says about where the run may go (event and board preruns share these fields). */
export interface ExecutionTargetPrerun {
	readonly can_execute_locally?: boolean;
	/** The board's execution mode. */
	readonly execution_mode?: string;
	/** The event's own execution mode (event preruns only). */
	readonly event_execution_mode?: string;
}

/**
 * The cloud when the backend always runs remotely, when there is no usable prerun answer, when this
 * device may not run the board, or when the board or the event is pinned to Remote; else this device.
 */
export function executionTargetOf(
	alwaysRemote: boolean,
	prerun: ExecutionTargetPrerun | null | undefined,
): ITemporaryUploadExecutionTarget {
	if (alwaysRemote || !prerun || !isRecord(prerun)) return "remote";
	const pinnedRemote =
		prerun.execution_mode === IExecutionMode.Remote ||
		prerun.event_execution_mode === IEventExecutionMode.Remote;
	return prerun.can_execute_locally === true && !pinnedRemote
		? "local"
		: "remote";
}

export interface ExecutionTargetBackend {
	readonly eventState: Pick<IEventState, "alwaysRemote" | "prerunEvent">;
}

/**
 * The target of an event's runs, from the same cached prerun the execution service reads
 * (`prerunSwr` under `prerunEventKey`). A backend without a prerun, or a prerun that fails, sends
 * the run to the cloud: an upload there is readable by every run, a local one only on this device.
 */
export async function resolveExecutionTarget(
	backend: ExecutionTargetBackend,
	appId: string,
	eventId: string,
): Promise<ITemporaryUploadExecutionTarget> {
	const events = backend.eventState;
	if (events.alwaysRemote === true) return "remote";
	const prerunEvent = events.prerunEvent;
	if (!prerunEvent) return "remote";
	try {
		const prerun = await prerunSwr(prerunEventKey(appId, eventId), () =>
			prerunEvent.call(events, appId, eventId),
		);
		return executionTargetOf(false, prerun);
	} catch {
		return "remote";
	}
}
