import { AVAILABILITY_STATUSES } from "../../lib/availability";
import { contextField } from "../../lib/bot-user-context";
import { isRecord } from "../../lib/response-shape";
import type {
	IInboxEntry,
	IUserContextRequest,
	IUserContextResponse,
	IUserContextRevoke,
} from "../../state/backend-state/inbox-state";
import { plainBody } from "./approval-entry";

export interface UserContextCardData {
	request: IUserContextRequest;
	answer?: IUserContextResponse["decision"];
	response?: IUserContextResponse;
	revoked?: boolean;
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

function finite(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value);
}

export function userContextResponseOf(
	entry: IInboxEntry,
): IUserContextResponse | undefined {
	const value = plainBody(entry)?.user_context_response;
	if (
		!isRecord(value) ||
		typeof value.request_id !== "string" ||
		value.subject_id !== entry.sender_id ||
		value.conversation_id !== entry.conversation_id ||
		typeof value.bot_id !== "string" ||
		!finite(value.captured_at) ||
		!finite(value.expires_at) ||
		!isRecord(value.values) ||
		!isRecord(value.unavailable)
	)
		return;
	if (
		value.decision !== "once" &&
		value.decision !== "session" &&
		value.decision !== "deny"
	)
		return;
	return value as unknown as IUserContextResponse;
}

export function userContextRevokeOf(
	entry: IInboxEntry,
): IUserContextRevoke | undefined {
	const value = plainBody(entry)?.user_context_revoke;
	if (
		!isRecord(value) ||
		typeof value.request_id !== "string" ||
		typeof value.bot_id !== "string" ||
		typeof value.subject_id !== "string" ||
		value.subject_id !== entry.sender_id ||
		value.conversation_id !== entry.conversation_id ||
		!finite(value.revoked_at)
	)
		return;
	return value as unknown as IUserContextRevoke;
}
