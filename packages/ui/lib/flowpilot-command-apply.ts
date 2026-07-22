import { getErrorMessage } from "./error-message";

const MAX_MODEL_FACING_FAILURES = 12;
const MAX_FAILURE_MESSAGE_CHARS = 500;

export interface FlowPilotCommandApplyFailure {
	/** Zero-based position in the model-authored BoardCommand queue, when known. */
	queueIndex?: number;
	phase: string;
	commandType: string;
	message: string;
}

export interface FlowPilotCommandApplyErrorOptions {
	requestedCommands: number;
	appliedCommands: number;
	failures: FlowPilotCommandApplyFailure[];
	refetched?: boolean;
	refreshError?: string;
}

function compactFailureMessage(message: string): string {
	const normalized = message.replace(/\s+/g, " ").trim();
	if (normalized.length <= MAX_FAILURE_MESSAGE_CHARS) return normalized;
	return `${normalized.slice(0, MAX_FAILURE_MESSAGE_CHARS - 1)}…`;
}

export function formatFlowPilotCommandApplyFailure(
	failure: FlowPilotCommandApplyFailure,
): string {
	const queueLocation =
		failure.queueIndex === undefined
			? failure.phase
			: `queue item ${failure.queueIndex + 1} (${failure.phase})`;
	return `${queueLocation}, ${failure.commandType}: ${compactFailureMessage(failure.message)}`;
}

export class FlowPilotCommandApplyError extends Error {
	readonly requestedCommands: number;
	readonly appliedCommands: number;
	readonly failures: FlowPilotCommandApplyFailure[];
	readonly refetched: boolean;
	readonly refreshError?: string;

	constructor(options: FlowPilotCommandApplyErrorOptions) {
		const visibleFailures = options.failures
			.slice(0, MAX_MODEL_FACING_FAILURES)
			.map(formatFlowPilotCommandApplyFailure);
		const omitted = options.failures.length - visibleFailures.length;
		const progress = `${options.appliedCommands}/${options.requestedCommands} queued command${
			options.requestedCommands === 1 ? "" : "s"
		} confirmed applied`;
		const details = visibleFailures.length
			? visibleFailures.map((failure) => `- ${failure}`).join("\n")
			: "- The queued command batch failed without a concrete diagnostic.";
		const omittedSuffix =
			omitted > 0
				? `\n- ${omitted} additional failure${omitted === 1 ? "" : "s"} omitted.`
				: "";
		const refreshSuffix = options.refetched
			? " The board was refetched and now reflects the authoritative current state."
			: options.refreshError
				? ` Board refetch also failed: ${compactFailureMessage(options.refreshError)}`
				: " Board refetch did not complete.";

		super(
			`FlowPilot could not apply the complete queued board change (${progress}).${refreshSuffix}\n${details}${omittedSuffix}`,
		);
		this.name = "FlowPilotCommandApplyError";
		this.requestedCommands = options.requestedCommands;
		this.appliedCommands = options.appliedCommands;
		this.failures = options.failures;
		this.refetched = options.refetched === true;
		this.refreshError = options.refreshError;
	}
}

function refetchResultError(result: unknown): unknown {
	if (!result || typeof result !== "object") return undefined;
	const error = (result as { error?: unknown }).error;
	return error ?? undefined;
}

/**
 * Always attempts a canonical board refetch before an apply failure escapes. The primary apply
 * diagnostics remain first even when recovery itself fails, so callers can play the actionable
 * queue error back to the model without replacing it with a secondary renderer/network error.
 */
export async function throwFlowPilotCommandApplyError(
	options: Omit<
		FlowPilotCommandApplyErrorOptions,
		"refetched" | "refreshError"
	>,
	refetch: () => Promise<unknown>,
): Promise<never> {
	let refreshError: string | undefined;
	try {
		const result = await refetch();
		const queryError = refetchResultError(result);
		if (queryError !== undefined) throw queryError;
	} catch (error) {
		refreshError = getErrorMessage(error, "Unknown board refetch error");
	}

	throw new FlowPilotCommandApplyError({
		...options,
		refetched: refreshError === undefined,
		refreshError,
	});
}

export async function executeFlowPilotCommandBatch<T>(options: {
	requestedCommands: number;
	alreadyAppliedCommands: number;
	expectedBatchCommands: number;
	phase: string;
	commandType: string;
	execute: () => Promise<unknown>;
	refetch: () => Promise<unknown>;
}): Promise<T[]> {
	let result: unknown;
	try {
		result = await options.execute();
	} catch (error) {
		await throwFlowPilotCommandApplyError(
			{
				requestedCommands: options.requestedCommands,
				appliedCommands: options.alreadyAppliedCommands,
				failures: [
					{
						phase: options.phase,
						commandType: options.commandType,
						message: getErrorMessage(
							error,
							"The backend rejected the queued command batch",
						),
					},
				],
			},
			options.refetch,
		);
	}

	if (
		!Array.isArray(result) ||
		result.length !== options.expectedBatchCommands
	) {
		await throwFlowPilotCommandApplyError(
			{
				requestedCommands: options.requestedCommands,
				appliedCommands: options.alreadyAppliedCommands,
				failures: [
					{
						phase: options.phase,
						commandType: options.commandType,
						message: `The backend confirmed ${Array.isArray(result) ? result.length : 0} of ${options.expectedBatchCommands} commands in this batch.`,
					},
				],
			},
			options.refetch,
		);
	}

	return result as T[];
}

export function flowPilotCommandApplyDiagnostics(error: unknown): string[] {
	if (error instanceof FlowPilotCommandApplyError) {
		const diagnostics = error.failures.map(formatFlowPilotCommandApplyFailure);
		if (error.refreshError) {
			diagnostics.push(`Board refetch failed: ${error.refreshError}`);
		}
		return diagnostics;
	}
	return [getErrorMessage(error, "Unknown queued board apply error")];
}
