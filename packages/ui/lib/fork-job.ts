import { isMissingResourceError } from "./api-error";
import type { IForkReport, IOnlineForkResponse } from "./schema/app/fork";

/**
 * `POST /apps/{id}/fork` answers `202` with this shape when the fork is too
 * large to finish inside the request; `GET /apps/fork/jobs/{job_id}` returns
 * the same shape until `status` is `DONE` and `report` is filled in.
 */
export interface IForkJobView {
	job_id: string;
	source_app_id: string;
	new_app_id: string;
	/** `QUEUED`, `RUNNING`, `DONE`, `FAILED` or `ABORTING`. */
	status: string;
	step: string;
	bytes_copied: number;
	objects_copied: number;
	report?: IForkReport;
	last_error?: string | null;
	created_at?: string;
	updated_at?: string;
	expires_at?: string;
}

export const FORK_JOB_POLL_INTERVAL_MS = 2_000;
export const FORK_JOB_POLL_MAX_INTERVAL_MS = 10_000;

/**
 * The `202` body carries a `job_id`; the synchronous `200` body never does.
 * Both carry `new_app_id`, so a caller that skips this check routes the user
 * into an app whose boards, events and pages do not exist yet.
 */
export function isForkJobView(
	response: IOnlineForkResponse | IForkJobView,
): response is IForkJobView {
	return typeof (response as IForkJobView).job_id === "string";
}

export interface AwaitForkJobOptions {
	pollIntervalMs?: number;
	maxPollIntervalMs?: number;
	sleep?: (ms: number) => Promise<void>;
}

function defaultSleep(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * A fork the server accepted with `202` finishes in the background; callers
 * keep their "Forking…" state while this polls the job, so they see the same
 * `IOnlineForkResponse` whether the server answered `200` or `202`.
 *
 * `fetchJob` is the client's own transport for `GET apps/fork/jobs/{job_id}`.
 */
export async function awaitForkJob(
	job: IForkJobView,
	fetchJob: (jobId: string) => Promise<IForkJobView>,
	options: AwaitForkJobOptions = {},
): Promise<IOnlineForkResponse> {
	const sleep = options.sleep ?? defaultSleep;
	const maxInterval =
		options.maxPollIntervalMs ?? FORK_JOB_POLL_MAX_INTERVAL_MS;
	let interval = options.pollIntervalMs ?? FORK_JOB_POLL_INTERVAL_MS;
	let current = job;
	for (;;) {
		if (current.status === "DONE") {
			if (current.report) {
				return { new_app_id: current.new_app_id, report: current.report };
			}
			throw new Error(`Fork ${current.job_id} finished without a report`);
		}
		if (current.status === "FAILED" || current.status === "ABORTING") {
			throw new Error(current.last_error ?? `Fork ${current.job_id} failed`);
		}
		await sleep(interval);
		interval = Math.min(interval * 1.5, maxInterval);
		try {
			current = await fetchJob(job.job_id);
		} catch (error) {
			if (isMissingResourceError(error)) {
				throw new Error(`Fork ${job.job_id} was aborted before it finished`);
			}
			throw error;
		}
	}
}

/** Normalizes either fork response into the finished fork the callers expect. */
export async function resolveOnlineFork(
	response: IOnlineForkResponse | IForkJobView,
	fetchJob: (jobId: string) => Promise<IForkJobView>,
	options?: AwaitForkJobOptions,
): Promise<IOnlineForkResponse> {
	if (!isForkJobView(response)) return response;
	return awaitForkJob(response, fetchJob, options);
}
