/**
 * Client half of the captured-FlowScript-apply-failure contract.
 *
 * Only clients that apply locally report through here — a web apply is recorded by the endpoint
 * that ran it, so the two paths never double-count. The vocabulary and the classification rule
 * mirror `packages/api/src/routes/flowscript.rs`; an outcome this file invents is rejected there.
 */

/** Authenticated, app-less capture route. Matches `POST /flowscript/apply-failure`. */
export const FLOWSCRIPT_APPLY_FAILURE_PATH = "flowscript/apply-failure";

/**
 * Who authored the source that was applied. FlowPilot goes through the same board pipeline as the
 * editor, so without this every agent attempt would land in the same bucket as a person's edit —
 * and the panel exists to show what *people* expected.
 */
export type FlowScriptApplyOrigin = "editor" | "agent";

/**
 * `error` — the apply threw; `blocked` — no commands came back, only diagnostics; `partial` — the
 * board changed but part of what the source asked for was skipped.
 */
export type FlowScriptApplyOutcome = "error" | "blocked" | "partial";

export interface IFlowScriptApplyFailureReport {
	readonly app_id: string;
	readonly board_id: string;
	readonly layer_id?: string;
	readonly outcome: FlowScriptApplyOutcome;
	readonly origin: FlowScriptApplyOrigin;
	/** Already redacted locally; the server redacts again regardless. */
	readonly flowscript: string;
	readonly error_message?: string;
	readonly diagnostics: string[];
	readonly corrections: string[];
	readonly command_count: number;
	readonly allow_deletions: boolean;
	readonly app_version?: string;
	readonly platform?: string;
}

/**
 * Classify a completed apply. `undefined` when the apply did exactly what the source asked, which
 * is the only case that is not worth capturing. Mirrors `outcome_for` on the server.
 */
export function flowScriptApplyOutcome(
	commandCount: number,
	diagnosticCount: number,
): FlowScriptApplyOutcome | undefined {
	if (diagnosticCount === 0) return undefined;
	return commandCount === 0 ? "blocked" : "partial";
}
