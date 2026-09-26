import type { IEventState } from "../../state/backend-state/event-state";

/** One onLoad run a page or dialog started, owned until it ends, is superseded or unmounts. */
export interface ILoadRun {
	readonly eventState: Pick<IEventState, "cancelExecution">;
	runId?: string;
	abandoned: boolean;
	cancelled: boolean;
}

export function createLoadRun(
	eventState: Pick<IEventState, "cancelExecution">,
): ILoadRun {
	return { eventState, abandoned: false, cancelled: false };
}

/** Marks the run abandoned and asks the host to stop it once its id is known. */
export function cancelLoadRun(run: ILoadRun): void {
	run.abandoned = true;
	if (!run.runId || run.cancelled) return;
	run.cancelled = true;
	const { eventState, runId } = run;
	// Hosts without a cancel path throw synchronously; the promise turns that into a rejection.
	void new Promise<void>((resolve) => {
		resolve(eventState.cancelExecution(runId));
	}).catch(() => {});
}

/** Records the run id; a run abandoned before its id arrived is cancelled now. */
export function adoptLoadRunId(run: ILoadRun, runId: string): boolean {
	run.runId ??= runId;
	if (!run.abandoned) return true;
	cancelLoadRun(run);
	return false;
}
