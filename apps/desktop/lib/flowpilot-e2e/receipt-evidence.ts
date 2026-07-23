import {
	type FlowScriptCompilerReceipt,
	type FlowScriptGenerationRunReceipt,
	isSuccessfulFlowScriptCheckReceipt,
	isSuccessfulFlowScriptCommitReceipt,
} from "@flow-like/flow-like-ui/lib/flowpilot/flowscript-generation-receipt";

export interface ExactSuccessfulCompilerPair {
	check: FlowScriptCompilerReceipt;
	commit: FlowScriptCompilerReceipt;
}

/**
 * Finds a compiler check/commit pair that proves the exact authored source was
 * validated and persisted. Draft identity and revision are mandatory so a
 * nearby successful check cannot accidentally bless a different commit.
 */
export function findExactSuccessfulCompilerPair(
	run: FlowScriptGenerationRunReceipt,
): ExactSuccessfulCompilerPair | null {
	if (run.outcome !== "ok" || run.persistedReadbackVerified !== true) {
		return null;
	}

	for (let index = run.compilerReceipts.length - 1; index >= 0; index -= 1) {
		const commit = run.compilerReceipts[index];
		if (!commit || !isSuccessfulFlowScriptCommitReceipt(commit)) continue;
		const check = run.compilerReceipts
			.slice(0, index)
			.findLast(
				(candidate) =>
					isSuccessfulFlowScriptCheckReceipt(candidate) &&
					candidate.source === commit.source &&
					Boolean(candidate.draftId) &&
					candidate.draftId === commit.draftId &&
					candidate.revision !== undefined &&
					candidate.revision === commit.revision,
			);
		if (check) return { check, commit };
	}

	return null;
}

export function authoredFlowScriptEvidence(
	runs: readonly FlowScriptGenerationRunReceipt[],
): { source?: string; status?: string; completion?: string } {
	for (let index = runs.length - 1; index >= 0; index -= 1) {
		const run = runs[index];
		if (!run) continue;
		const pair = findExactSuccessfulCompilerPair(run);
		if (!pair?.commit.source) continue;
		const candidate = [...run.candidates]
			.reverse()
			.find((entry) => entry.source === pair.commit.source);
		return {
			source: pair.commit.source,
			status: pair.commit.status ?? candidate?.status,
			completion: candidate?.completion,
		};
	}

	return {};
}
