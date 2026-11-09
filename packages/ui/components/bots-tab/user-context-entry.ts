import { AVAILABILITY_STATUSES } from "../../lib/availability";
import { contextField } from "../../lib/bot-user-context";
import { isRecord } from "../../lib/response-shape";
import type {
	IInboxEntry,
	IUserContextRequest,
	IUserContextResponse,
} from "../../state/backend-state/inbox-state";
import { plainBody } from "./approval-entry";

export interface UserContextCardData {
	request: IUserContextRequest;
	answer?: IUserContextResponse["decision"];
}

export function userContextRequestOf(
	entry: IInboxEntry,
): IUserContextRequest | undefined {
	const value = plainBody(entry)?.user_context_request;
	if (
		!isRecord(value) ||
		value.id !== entry.message_id ||
		value.bot_id !== entry.sender_id ||
		value.conversation_id !== entry.conversation_id
	)
		return;
	if (
		typeof value.subject_id !== "string" ||
		!value.subject_id ||
		typeof value.purpose !== "string" ||
		!value.purpose.trim() ||
		!Array.isArray(value.fields) ||
		value.fields.length === 0 ||
		value.fields.length > 8 ||
		new Set(value.fields).size !== value.fields.length ||
		!value.fields.every(contextField) ||
		(value.precision !== "approximate" && value.precision !== "precise") ||
		(value.audience !== "direct" && value.audience !== "group") ||
		!Array.isArray(value.audience_member_ids) ||
		!value.audience_member_ids.every((id) => typeof id === "string") ||
		typeof value.created_at !== "number" ||
		!Number.isFinite(value.created_at) ||
		typeof value.expires_at !== "number" ||
		!Number.isFinite(value.expires_at) ||
		value.expires_at <= value.created_at ||
		value.expires_at - value.created_at > 300_000
	)
		return;
	if (value.status_change !== undefined) {
		const change = value.status_change;
		if (
			!isRecord(change) ||
			Object.keys(change).some(
				(key) => key !== "status" && key !== "duration_minutes",
			) ||
			!AVAILABILITY_STATUSES.includes(change.status as never) ||
			!Number.isInteger(change.duration_minutes) ||
			Number(change.duration_minutes) < 1 ||
			Number(change.duration_minutes) > 240 ||
			value.fields.length !== 1 ||
			value.fields[0] !== "availability"
		)
			return;
	}
	return value as unknown as IUserContextRequest;
}

export function userContextResponseOf(
	entry: IInboxEntry,
):
	| Pick<
			IUserContextResponse,
			"request_id" | "subject_id" | "bot_id" | "decision"
	  >
	| undefined {
	const value = plainBody(entry)?.user_context_response;
	if (
		!isRecord(value) ||
		typeof value.request_id !== "string" ||
		value.subject_id !== entry.sender_id ||
		value.conversation_id !== entry.conversation_id ||
		typeof value.bot_id !== "string"
	)
		return;
	if (
		value.decision !== "once" &&
		value.decision !== "session" &&
		value.decision !== "deny"
	)
		return;
	return {
		request_id: value.request_id,
		subject_id: entry.sender_id,
		bot_id: value.bot_id,
		decision: value.decision,
	};
}
