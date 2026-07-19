import type {
	FlowIrCommitDispositionResult,
	FlowIrCommitToken,
} from "../../lib/schema/copilot";

/**
 * Preserve both user-owned layers of a delegated request without admitting host orchestration
 * suffixes into routing or acceptance checks. The outer request remains authoritative; the
 * specialist instruction may narrow or expand how that request is implemented.
 */
export function composeDelegatedRawUserPrompt(
	sourceUserPrompt: string | undefined,
	specialistInstruction: string,
): string {
	const source = sourceUserPrompt?.trim() ?? "";
	const instruction = specialistInstruction.trim();
	if (!source) return instruction;
	if (!instruction || instruction === source) return source;
	return `${source}\n\nDelegated specialist request:\n${instruction}`;
}

/**
 * A response can arrive after cancellation or a board/session switch. Release any native compiled
 * workflow claim before the caller rejects that stale response, so the invisible result cannot
 * strand a retained review token.
 */
export async function releaseReturnedFlowIrCommitBeforeStaleResponse(
	responseBelongsToActiveRequest: boolean,
	token: FlowIrCommitToken | undefined,
	dismiss: (token: FlowIrCommitToken) => Promise<FlowIrCommitDispositionResult>,
): Promise<FlowIrCommitDispositionResult | undefined> {
	if (responseBelongsToActiveRequest || !token) return undefined;
	return await dismiss(token);
}
