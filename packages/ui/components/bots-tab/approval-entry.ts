import { isRecord } from "../../lib/response-shape";
import {
	BOT_SECRET_NAME_PATTERN,
	type IBotApprovalItemDecision,
} from "../../state/backend-state/bot-state";
import type {
	IInboxApproval,
	IInboxApprovalItem,
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

function decisionOf(value: unknown): ApprovalDecision | undefined {
	return typeof value === "string" && DECISIONS.includes(value)
		? (value as ApprovalDecision)
		: undefined;
}

function callsOf(value: unknown): number {
	return typeof value === "number" && Number.isInteger(value) && value > 1
		? value
		: 1;
}

function itemOf(value: unknown): IInboxApprovalItem | undefined {
	if (!isRecord(value)) return undefined;
	const { args_hash, tool } = value;
	if (typeof args_hash !== "string" || !args_hash || typeof tool !== "string")
		return undefined;
	return {
		args_hash,
		tool,
		summary: text(value.summary) ?? "",
		scope: text(value.scope) ?? "",
		calls: callsOf(value.calls),
		...(value.after_web_content === true ? { after_web_content: true } : {}),
		...secretField(value.secret),
		...inviteField(value.invite),
	};
}

/** The well-formed items of a card, each hash once. */
function itemsOf(value: unknown): IInboxApprovalItem[] {
	if (!Array.isArray(value)) return [];
	const seen = new Set<string>();
	return value.flatMap((raw) => {
		const item = itemOf(raw);
		if (!item || seen.has(item.args_hash)) return [];
		seen.add(item.args_hash);
		return [item];
	});
}

/**
 * The card an entry carries. A card with one item reads as that item, so it renders exactly like
 * a card written before batches existed.
 */
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
	const card: IInboxApproval = {
		id,
		turn_id,
		tool,
		summary: typeof summary === "string" ? summary : "",
		scope: typeof scope === "string" ? scope : "",
		args_hash: text(body.args_hash) ?? "",
		requester_id: text(body.requester_id),
		requester_name: text(body.requester_name),
		room: text(body.room),
		...(isRecord(body.configuration_changes)
			? { configuration_changes: body.configuration_changes }
			: {}),
		...(body.after_web_content === true ? { after_web_content: true } : {}),
		...secretField(body.secret),
		...inviteField(body.invite),
	};
	const items = itemsOf(body.items);
	if (items.length > 1) return { ...card, items };
	const [only] = items;
	if (!only) return card;
	const { calls: _calls, ...single } = only;
	return {
		...card,
		...single,
		after_web_content: only.after_web_content,
		secret: only.secret,
		invite: only.invite,
	};
}

function inviteField(value: unknown): Pick<IInboxApproval, "invite"> {
	if (!isRecord(value)) return {};
	const { person, reason } = value;
	if (typeof person !== "string" || !person || typeof reason !== "string")
		return {};
	return { invite: { person, reason } };
}

function secretField(value: unknown): Pick<IInboxApproval, "secret"> {
	if (!isRecord(value)) return {};
	const { name, description, replace } = value;
	if (
		typeof name !== "string" ||
		!BOT_SECRET_NAME_PATTERN.test(name) ||
		typeof description !== "string"
	)
		return {};
	return {
		secret: { name, description, ...(replace === true ? { replace } : {}) },
	};
}

function itemDecisionsOf(value: unknown): IBotApprovalItemDecision[] {
	if (!Array.isArray(value)) return [];
	return value.flatMap((raw) => {
		if (!isRecord(raw)) return [];
		const decision = decisionOf(raw.decision);
		const argsHash = text(raw.args_hash);
		return decision && argsHash ? [{ args_hash: argsHash, decision }] : [];
	});
}

/** The owner's answer an entry carries, if it is one. */
export function approvalResponseOf(
	entry: IInboxEntry,
): IInboxApprovalResponse | undefined {
	const body = plainBody(entry)?.approval_response;
	if (!isRecord(body)) return undefined;
	const approvalId = text(body.approval_id);
	const decision = decisionOf(body.decision);
	if (!approvalId || !decision) return undefined;
	const items = itemDecisionsOf(body.items);
	return {
		approval_id: approvalId,
		decision,
		...(items.length > 0 ? { items } : {}),
	};
}

/** How many calls the card in this entry asks about, identical ones each; 0 when it is no card. */
export function waitingCallsOf(entry: IInboxEntry | null): number {
	const approval = entry?.kind === "message" ? approvalOf(entry) : undefined;
	if (!approval) return 0;
	return approval.items
		? approval.items.reduce((sum, item) => sum + callsOf(item.calls), 0)
		: 1;
}

/** Whether this entry is an approval card that no answer follows yet (judged from the entry alone). */
export function isApprovalCard(entry: IInboxEntry | null): boolean {
	return (
		entry !== null &&
		entry.kind === "message" &&
		approvalOf(entry) !== undefined
	);
}
