import { FLOWPILOT_DEBUG_ENABLED } from "../../lib/flowpilot-debug";
import type { FlowIrCommitToken } from "../../lib/schema/copilot";
import {
	type AgentDebugOutcome,
	type AgentGenerationReviewDisposition,
	agentGenerationReviewDispositionEvent,
	beginAgentGenerationMetrics,
	createAgentDebugStreamRecorder,
	finalizeAgentGenerationMetrics,
	recordAgentGenerationMetricEvent,
} from "../../state/global-chat/agent-debug-report";

/**
 * Aggregate-only generation telemetry for the direct FlowPilot surface.
 *
 * A successful generation that returns a reviewable workflow stays open until Apply/Dismiss so
 * queueing cannot be mistaken for acceptance. The recorder deliberately runs in metric-only mode:
 * prompts, tool arguments, generated source, and board identifiers are never retained here.
 */
export class FlowPilotGenerationMetricsRun {
	readonly runKey: string;
	private readonly recorder: ReturnType<typeof createAgentDebugStreamRecorder>;
	private outcome: AgentDebugOutcome = "running";
	private awaitingReview = false;
	private finalized = false;
	private finalBoardNodeCount: number | undefined;
	private failure: { code?: unknown; message?: unknown } | undefined;

	constructor(runKey: string, startedAtMs = Date.now()) {
		this.runKey = runKey;
		beginAgentGenerationMetrics(runKey, startedAtMs);
		this.recorder = createAgentDebugStreamRecorder({
			scope: "main",
			requestId: runKey,
			enabled: false,
			record: (event) => recordAgentGenerationMetricEvent(runKey, event),
		});
	}

	push(chunk: string) {
		if (!this.finalized) this.recorder.push(chunk);
	}

	/** Finish the stream. Reviewable work remains open until a host disposition is recorded. */
	finish(
		outcome: AgentDebugOutcome,
		awaitingReview = false,
		finalBoardNodeCount?: number,
		failure?: { code?: unknown; message?: unknown },
	) {
		if (this.finalized) return;
		this.recorder.flush();
		this.outcome = outcome;
		this.awaitingReview = awaitingReview;
		this.failure = failure ?? this.failure;
		this.observeFinalBoardNodeCount(finalBoardNodeCount);
		if (!awaitingReview) this.finalize();
	}

	/** Record the host-side fate of a queued workflow and publish the completed aggregate. */
	disposeReview(
		disposition: AgentGenerationReviewDisposition,
		token?: FlowIrCommitToken,
		finalBoardNodeCount?: number,
		reason?: unknown,
	) {
		if (this.finalized) return;
		if (disposition === "error" && this.outcome === "ok") {
			this.outcome = "error";
		}
		recordAgentGenerationMetricEvent(
			this.runKey,
			agentGenerationReviewDispositionEvent({
				requestId: this.runKey,
				disposition,
				draftId: token?.draft_id,
				revision: token?.revision,
				claimId: token?.claim_id,
				reason,
			}),
		);
		this.awaitingReview = false;
		this.observeFinalBoardNodeCount(finalBoardNodeCount);
		this.finalize();
	}

	/** Retain only the aggregate count; no board id, node id, or generated source is recorded. */
	observeFinalBoardNodeCount(count: number | undefined) {
		if (Number.isSafeInteger(count) && (count ?? -1) >= 0) {
			this.finalBoardNodeCount = count;
		}
	}

	/** Fail closed if a surface disappears before its review can be resolved. */
	abandon(outcome: AgentDebugOutcome = "cancelled", reason?: unknown) {
		if (this.finalized) return;
		this.outcome = outcome;
		if (this.awaitingReview) {
			this.disposeReview("stale", undefined, undefined, reason);
			return;
		}
		this.finalize();
	}

	private finalize() {
		if (this.finalized) return;
		this.finalized = true;
		finalizeAgentGenerationMetrics(this.runKey, this.outcome, {
			publish: !FLOWPILOT_DEBUG_ENABLED,
			finalBoardNodeCount: this.finalBoardNodeCount,
			failure: this.failure,
		});
	}
}
