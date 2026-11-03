import type { IBotScopes } from "./bot-state";

export type ConversationKind = "direct" | "group";
export type ConversationEncryption = "plain" | "e2e";
export type InboxPrincipalKind = "user" | "bot";
export type InboxEntryKind =
	| "message"
	| "reaction"
	| "edit"
	| "delete"
	| "topic"
	| "member"
	| "trigger";

/** At most this many members, so one fan-out stays far below the database's transaction cap. */
export const MAX_CONVERSATION_MEMBERS = 50;

export interface IInboxPlainEnvelope<Body = unknown> {
	v: 1;
	enc: "none";
	body: Body;
}

/** Sealed to one recipient's inbox key; only that recipient can open it. */
export interface IInboxSealedEnvelope {
	v: 1;
	enc: string;
	key_id: string;
	enc_key: string;
	ct: string;
}

export type IInboxEnvelope = IInboxPlainEnvelope | IInboxSealedEnvelope;

export interface IInboxMessageBody {
	text: string;
	reply_to?: string;
	/** Set by a chat that answers this message itself; hosts leave it alone meanwhile. */
	interactive?: { client_run_id: string };
	approval?: IInboxApproval;
	approval_response?: IInboxApprovalResponse;
}

/** The card a bot posts when a turn needs its owner to allow a call; `scope` is comma-joined, empty when the call is never remembered. */
export interface IInboxApproval {
	id: string;
	turn_id: string;
	tool: string;
	summary: string;
	scope: string;
	args_hash: string;
	requester_id?: string;
	requester_name?: string;
	room?: string;
}

export interface IInboxApprovalResponse {
	approval_id: string;
	decision: "once" | "always" | "deny";
}

export interface IInboxReactionBody {
	target: string;
	emoji: string;
	op: "add" | "remove";
}

export interface IInboxEditBody {
	target: string;
	text: string;
}

export interface IInboxDeleteBody {
	target: string;
}

export interface IInboxMemberBody {
	op: "join" | "leave";
	principal_id: string;
	by: string;
}

export interface IConversationMember {
	/** A user sub or a bot id. */
	principal_id: string;
	kind: InboxPrincipalKind;
	added_by: string | null;
	joined_at: string;
	/** A bot's room limits; only its owner sees them. */
	limits?: IBotRoomLimits | null;
}

/** What a bot may do in one room, on top of its own restrictions; each field only narrows. */
export interface IBotRoomLimits {
	/** Most permissions (role bits) the bot holds in the room; omitted adds no limit. */
	mask?: number | null;
	/** Apps the bot may use in the room; omitted adds no limit. */
	apps?: string[] | null;
	/** Further scopes per app id. */
	scopes?: Record<string, IBotScopes>;
}

/** Someone invited to a group who has not joined yet. */
export interface IPendingMember {
	principal_id: string;
	kind: InboxPrincipalKind;
	invited_by: string | null;
	invited_at: string | null;
	status: "pending" | "declined";
}

/** One principal's copy of one event. Every copy of the same event shares `message_id`. */
export interface IInboxEntry {
	id: string;
	message_id: string;
	conversation_id: string;
	sender_id: string;
	kind: InboxEntryKind;
	envelope: IInboxEnvelope;
	created_at: string;
	/** Opaque position; pass it back as `before` or `after`. */
	cursor: string;
	/** The event's position in the conversation (`#seq` in a bot's history). */
	seq?: number | null;
}

export interface IConversation {
	id: string;
	kind: ConversationKind;
	title: string | null;
	encryption: ConversationEncryption;
	created_by: string;
	created_at: string;
	last_activity_at: string;
	/** Active members only. */
	members: IConversationMember[];
	/** Invitations nobody accepted yet; they read and receive nothing. */
	pending?: IPendingMember[];
	unread_count: number;
	/** Newest `message` entry in the reader's inbox. */
	last_entry: IInboxEntry | null;
	/** Lives only on this device, between you and local bots, until it moves to an account. */
	local?: boolean;
}

export interface ICreateConversationRequest {
	kind: ConversationKind;
	title?: string | null;
	/** Everyone except the caller. A direct conversation takes exactly one. */
	members: string[];
	encryption?: ConversationEncryption;
}

export interface ISendInboxEntryRequest {
	/** Chosen by the sender; sending the same id again returns the first result. */
	message_id: string;
	kind: Exclude<InboxEntryKind, "member" | "trigger">;
	/** Plain conversations. */
	envelope?: IInboxPlainEnvelope;
	/** End-to-end encrypted conversations: one sealed envelope per active member. */
	envelopes?: Record<string, IInboxSealedEnvelope>;
}

export interface IInboxEntriesPage {
	/** Newest first; by position (`from_seq`, `to_seq`, `around_seq`) oldest first. */
	entries: IInboxEntry[];
	next_before: string | null;
	/** Reading by position: more entries follow; pass this as `from_seq`. */
	next_from_seq?: number | null;
	/** Reading by position: more entries precede; pass this as `to_seq`. */
	next_to_seq?: number | null;
}

export interface IInboxEntriesQuery {
	/** Read a bot's inbox you own instead of your own. */
	as?: string;
	before?: string;
	after?: string;
	limit?: number;
	/** By position: entries from this `seq` on, oldest first. */
	from_seq?: number;
	/** By position: entries up to this `seq`; alone, the newest `limit` of them. */
	to_seq?: number;
	/** By position: the entry at this `seq` with entries around it. */
	around_seq?: number;
	before_count?: number;
	after_count?: number;
}

/** A summary of part of a bot's history; the chat's jump list. */
export interface IConversationSegment {
	id: string;
	author_id: string;
	level: number;
	seq_from: number;
	seq_to: number;
	t_from: string;
	t_to: string;
	title: string | null;
	summary: string;
	parent_id: string | null;
	version: number;
	created_at: string;
}

export interface IConversationSegmentList {
	segments: IConversationSegment[];
	next_after: string | null;
}

export interface IInboxSegmentsQuery {
	/** Read as one of your bots. */
	as?: string;
	level?: number;
	after?: string;
	limit?: number;
}

/** One message of a bot's earlier chat, moved into its inbox. */
export interface IInboxImportEntry {
	/** Kept as the inbox message id, so importing it again adds nothing. */
	message_id: string;
	sender: "owner" | "bot";
	envelope: IInboxPlainEnvelope;
	created_at: string;
}

export interface IInboxImportResult {
	imported: number;
	skipped: number;
}

export interface IInboxKey {
	principal_id: string;
	key_id: string;
	algorithm: string;
	public_key: string;
	created_at: string;
}

export interface IRegisterInboxKeyRequest {
	/** Omit for your own key; a bot id you own registers that bot's key. */
	principal_id?: string;
	key_id: string;
	algorithm: string;
	public_key: string;
}

/** An invitation to a room for you or for one of your bots. */
export interface IInvitation {
	/** `{conversation_id}:{principal_id}`. */
	id: string;
	conversation_id: string;
	title: string | null;
	/** You, or the bot of yours that is invited. */
	principal_id: string;
	kind: InboxPrincipalKind;
	bot_name: string | null;
	invited_by: string | null;
	invited_at: string | null;
}

/** `add_now`: your own bot joins at once. `invite`: they (or the bot's owner) must accept. */
export type InviteMode = "add_now" | "invite";

export interface IInvitable {
	principal_id: string;
	kind: InboxPrincipalKind;
	name: string | null;
	description: string | null;
	owner_id: string | null;
	mode: InviteMode;
	/** Set when the room is known and the principal is already in or invited to it. */
	state: "member" | "invited" | null;
}

export interface IInviteRequest {
	principal_ids: string[];
	/** Room limits for your own bots as they join. */
	bot_limits?: Record<string, IBotRoomLimits>;
}

export interface IInviteResponse {
	added: string[];
	invited: string[];
	conversation: IConversation;
}

export interface IInboxState {
	/** Newest activity first. `as` reads one of your bots' inboxes. */
	listConversations(as?: string): Promise<IConversation[]>;
	getConversation(conversationId: string, as?: string): Promise<IConversation>;
	/** A direct conversation that already exists is returned instead of a new one. */
	createConversation(
		request: ICreateConversationRequest,
	): Promise<IConversation>;
	renameConversation(
		conversationId: string,
		title: string,
	): Promise<IConversation>;
	addMembers(conversationId: string, members: string[]): Promise<IConversation>;
	removeMember(conversationId: string, principalId: string): Promise<void>;
	listEntries(
		conversationId: string,
		query?: IInboxEntriesQuery,
	): Promise<IInboxEntriesPage>;
	/** Pending invitations for you and your bots, newest first. */
	listInvitations(): Promise<IInvitation[]>;
	/** Exactly who the server lets you bring into a room. */
	listInvitable(query?: {
		q?: string;
		conversationId?: string;
	}): Promise<IInvitable[]>;
	invite(
		conversationId: string,
		request: IInviteRequest,
	): Promise<IInviteResponse>;
	/** For a bot's invitation the owner sets the room's limits. */
	acceptInvitation(
		invitation: IInvitation,
		limits?: IBotRoomLimits | null,
	): Promise<void>;
	declineInvitation(invitation: IInvitation): Promise<void>;
	cancelInvitation(conversationId: string, principalId: string): Promise<void>;
	setMemberLimits(
		conversationId: string,
		botId: string,
		limits: IBotRoomLimits | null,
	): Promise<void>;
	sendEntry(
		conversationId: string,
		request: ISendInboxEntryRequest,
	): Promise<IInboxEntry>;
	/**
	 * Moves messages of a bot's earlier chat into its direct conversation, oldest first. Messages
	 * already there (same id) are skipped.
	 */
	importHistory?(
		botId: string,
		conversationId: string,
		entries: IInboxImportEntry[],
	): Promise<IInboxImportResult>;
	/** The summaries of a conversation's history, oldest range first. */
	listSegments(
		conversationId: string,
		query?: IInboxSegmentsQuery,
	): Promise<IConversationSegmentList>;
	markRead(
		conversationId: string,
		upTo?: string,
	): Promise<{ last_read_at: string }>;
	getKeys(principalIds: string[]): Promise<IInboxKey[]>;
	registerKey(request: IRegisterInboxKeyRequest): Promise<IInboxKey>;
	/** Retires a key; pass one of your bots' ids to retire that bot's key. */
	revokeKey(keyId: string, principalId?: string): Promise<void>;
}

/** The by-position part of an entries query as URL parameters; unset fields are left out. */
export function inboxPositionParams(
	query: IInboxEntriesQuery,
): Record<string, string | undefined> {
	return {
		from_seq: query.from_seq?.toString(),
		to_seq: query.to_seq?.toString(),
		around_seq: query.around_seq?.toString(),
		before_count: query.before_count?.toString(),
		after_count: query.after_count?.toString(),
	};
}
