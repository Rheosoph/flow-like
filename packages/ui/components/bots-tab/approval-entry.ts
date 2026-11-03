import { isRecord } from "../../lib/response-shape";
import type {
	IInboxApproval,
	IInboxApprovalResponse,
	IInboxEntry,
} from "../../state/backend-state/inbox-state";

export type ApprovalDecision = IInboxApprovalResponse["decision"];

export function plainBody(entry: IInboxEntry): Record<string, unknown> | null {
	const envelope: unknown = entry.envelope;
	if (!isRecord(envelope) || envelope.enc !== "none") return null;
	return isRecord(envelope.body) ? envelope.body : null;
}

function text(value: unknown): string | undefined {
	return typeof value === "string" ? value : undefined;
}

const DECISIONS: readonly string[] = ["once", "always", "deny"];

export function approvalOf(entry: IInboxEntry): IInboxApproval | undefined {
	const body = plainBody(entry)?.approval;
	if (!isRecord(body)) return undefined;
	const { id, turn_id, tool, summary, scope } = body;
	if (
		typeof id !== "string" ||
		typeof turn_id !== "string" ||
		typeof tool !== "string"
	)
		return undefined;
	return {
		id,
		turn_id,
		tool,
		summary: typeof summary === "string" ? summary : "",
		scope: typeof scope === "string" ? scope : "",
		args_hash: text(body.args_hash) ?? "",
	};
}

/** The owner's answer an entry carries, if it is one. */
export function approvalResponseOf(
	entry: IInboxEntry,
): IInboxApprovalResponse | undefined {
	const body = plainBody(entry)?.approval_response;
	if (!isRecord(body)) return undefined;
	const approvalId = text(body.approval_id);
	const decision = text(body.decision);
	if (!approvalId || !decision || !DECISIONS.includes(decision))
		return undefined;
	return { approval_id: approvalId, decision: decision as ApprovalDecision };
}

/** Whether this entry is an approval card that no answer follows yet (judged from the entry alone). */
export function isApprovalCard(entry: IInboxEntry | null): boolean {
	return (
		entry !== null &&
		entry.kind === "message" &&
		approvalOf(entry) !== undefined
	);
}
