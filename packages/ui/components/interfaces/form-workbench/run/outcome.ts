/*
 * How a run ended, decided by the form itself on every transport (PLAN §3.5). The rules are
 * tried in order; the first that matches wins. `completed.status` is authoritative wherever a
 * terminal event arrived; the promise's way of ending only decides when none did.
 */
import { isTransportFailure } from "../../../../lib/api-error";
import { getErrorMessage } from "../../../../lib/error-message";
import type {
	FailureKind,
	NotStartedReason,
	OutcomeInput,
	OutcomeOf,
	RunOutcome,
	TerminalSignals,
} from "../contracts";

type JsonRecord = Record<string, unknown>;
type Rule = (input: OutcomeInput) => RunOutcome | null;

/** The API's quota resource for "every place of the plan is taken" (M5). */
export const NO_PLACE_RESOURCE = "concurrent_cloud_executions";

/** Thrown by the execution service when its pre-run prompt is cancelled or torn down. */
const PROMPT_CANCELLED: ReadonlySet<string> = new Set([
	"Execution cancelled: runtime variables not configured",
	"Execution cancelled: the execution service unmounted while the run waited for a prompt",
]);
/** Thrown by the desktop when the person declines the computer-automation consent. */
const CONSENT_DECLINED: ReadonlySet<string> = new Set([
	"Computer automation was not approved for this event.",
]);
const FAILURE_BY_STATUS: ReadonlyMap<number, FailureKind> = new Map([
	[401, "permission"],
	[402, "quota"],
	[403, "permission"],
]);
const ABORT_NAMES: ReadonlySet<string> = new Set([
	"AbortError",
	"TimeoutError",
]);
/** `ILogMetadata.log_level` from Error up means the run failed (fallback without a terminal event). */
const FAILED_LOG_LEVEL = 3;

const SUCCEEDED: RunOutcome = { kind: "succeeded" };
const STOPPED: RunOutcome = { kind: "stopped" };

const isRecord = (value: unknown): value is JsonRecord =>
	typeof value === "object" && value !== null;

const notStarted = (reason: NotStartedReason): RunOutcome => ({
	kind: "notStarted",
	reason,
});

const failed = (
	failure: FailureKind,
	message: string | null,
	detail: string | null,
): RunOutcome => ({ kind: "failed", failure, message, detail });

function textOf(value: unknown) {
	return typeof value === "string" && value.trim() ? value.trim() : null;
}

function statusOf(error: unknown) {
	return isRecord(error) && typeof error.status === "number"
		? error.status
		: null;
}

/** The raw text of any rejection: Error, Tauri `{ error }` object, plain string. */
function rawMessageOf(error: unknown) {
	return textOf(getErrorMessage(error, ""));
}

/** What may be shown: the API's own text without its `[CODE]` prefix, else the raw text. */
function shownMessageOf(error: unknown) {
	return (
		(isRecord(error) && textOf(error.serverMessage)) || rawMessageOf(error)
	);
}

function isAbortError(error: unknown) {
	return isRecord(error) && ABORT_NAMES.has(String(error.name));
}

/** A start the API refused because every place of the plan is taken (402 · concurrent_cloud_executions). */
export function isNoPlaceError(error: unknown) {
	if (!isRecord(error)) return false;
	const quota = isRecord(error.quota) ? error.quota : null;
	return (
		quota?.resource === NO_PLACE_RESOURCE &&
		(error.status === 402 || error.code === "PLAN_LIMIT_EXCEEDED")
	);
}

const failureKindOf = (error: unknown): FailureKind => {
	if (isRecord(error) && error.isOAuthError === true) return "oauth";
	const status = statusOf(error);
	const byStatus = status === null ? undefined : FAILURE_BY_STATUS.get(status);
	if (byStatus) return byStatus;
	if (isAbortError(error) || isTransportFailure(error)) return "network";
	return "flow";
};

/**
 * A rejected run promise on its own, without what the stream said: a cancelled pre-run prompt or a
 * declined consent never started; a refused place is no failure; everything else failed.
 */
export const classifyError = (error: unknown): RunOutcome => {
	const raw = rawMessageOf(error);
	if (raw && PROMPT_CANCELLED.has(raw)) return notStarted("promptCancelled");
	if (raw && CONSENT_DECLINED.has(raw)) return notStarted("declined");
	if (isNoPlaceError(error)) return { kind: "noPlace" };
	return failed(failureKindOf(error), shownMessageOf(error), raw);
};

/** "Execution failed · status failed · log level 4": the line behind "Details". */
function terminalDetail(terminal: TerminalSignals) {
	const parts = [
		terminal.errorMessage,
		terminal.completedStatus ? `status ${terminal.completedStatus}` : null,
		terminal.rejectedStage === null
			? null
			: `rejected:${terminal.rejectedStage}`,
		terminal.logLevel === null ? null : `log level ${terminal.logLevel}`,
	].filter((part): part is string => Boolean(part));
	return parts.length ? parts.join(" · ") : null;
}

function streamFailure(failure: FailureKind, terminal: TerminalSignals) {
	return failed(
		failure,
		textOf(terminal.errorMessage),
		terminalDetail(terminal),
	);
}

function rejectedError(input: OutcomeInput) {
	return input.settlement.kind === "rejected" ? input.settlement.error : null;
}

function resolvedEmpty(input: OutcomeInput) {
	return input.settlement.kind === "resolved" && input.settlement.meta == null;
}

const promptRule: Rule = (input) => {
	if (input.settlement.kind !== "rejected") return null;
	const outcome = classifyError(input.settlement.error);
	return outcome.kind === "notStarted" ? outcome : null;
};

const declinedRule: Rule = (input) =>
	resolvedEmpty(input) && input.eventCount === 0 && !input.terminal.runInitiated
		? notStarted(input.stopRequested ? "removedFromQueue" : "declined")
		: null;

const noPlaceRule: Rule = (input) => {
	if (!isNoPlaceError(rejectedError(input))) return null;
	return input.stopRequested
		? notStarted("removedFromQueue")
		: { kind: "noPlace" };
};

const stopRule: Rule = (input) => (input.stopRequested ? STOPPED : null);

const cancelledRule: Rule = (input) =>
	input.terminal.completedStatus === "cancelled" ? STOPPED : null;

const rejectedStageRule: Rule = (input) =>
	input.terminal.rejectedStage === null
		? null
		: streamFailure("rejected", input.terminal);

const timeoutRule: Rule = (input) =>
	input.terminal.completedStatus === "timeout"
		? streamFailure("timeout", input.terminal)
		: null;

const streamErrorRule: Rule = (input) =>
	input.terminal.errorMessage !== null ||
	input.terminal.completedStatus === "failed"
		? streamFailure("flow", input.terminal)
		: null;

const rejectionRule: Rule = (input) =>
	input.settlement.kind === "rejected"
		? classifyError(input.settlement.error)
		: null;

const completedRule: Rule = (input) =>
	input.terminal.completedStatus === "completed" ? SUCCEEDED : null;

const logLevelRule: Rule = (input) => {
	const meta =
		input.settlement.kind === "resolved" ? input.settlement.meta : null;
	if (!meta || !(meta.log_level >= FAILED_LOG_LEVEL)) return null;
	const run = textOf(meta.run_id);
	return failed(
		"flow",
		null,
		`log level ${meta.log_level}${run ? ` · run ${run}` : ""}`,
	);
};

/** The stream ended without a terminal event after the run showed signs of life: nobody knows how it ended. */
const lostRule: Rule = (input) =>
	resolvedEmpty(input) && (input.terminal.runInitiated || input.eventCount > 0)
		? { kind: "unknown" }
		: null;

const RULES: readonly Rule[] = [
	promptRule,
	declinedRule,
	noPlaceRule,
	stopRule,
	cancelledRule,
	rejectedStageRule,
	timeoutRule,
	streamErrorRule,
	rejectionRule,
	completedRule,
	logLevelRule,
	lostRule,
];

/** First match wins (PLAN §3.5); a run nothing speaks against succeeded. */
export const outcomeOf: OutcomeOf = (input) => {
	for (const rule of RULES) {
		const outcome = rule(input);
		if (outcome) return outcome;
	}
	return SUCCEEDED;
};
