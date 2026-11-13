import type { IChatUsageStat } from "../../lib/llm/usage";
import type { IRealtimeAccess } from "../../lib/realtime/types";
import type {
	BotApprovalDecision,
	IBotApprovalItemDecision,
	IBotNoteRef,
	IBotScopes,
} from "./bot-state";

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
	| "trigger"
	| "activity"
	| "call"
	| "transcript"
	| "room";

/** At most this many members, so one fan-out stays far below the database's transaction cap. */
export const MAX_CONVERSATION_MEMBERS = 50;

export interface IInboxPlainEnvelope<Body = unknown> {
	v: 1;
	enc: "none";
	body: Body;
}

/** Sealed to one recipient: to one key, or to each key of their devices. Only they open it. */
export interface IInboxSealedEnvelope {
	v: 1;
	enc: string;
	ct: string;
	key_id?: string;
	enc_key?: string;
	keys?: { key_id: string; enc_key: string; ck: string }[];
}

export type IInboxEnvelope = IInboxPlainEnvelope | IInboxSealedEnvelope;

/** A file in a bot's workspace, or a conversation file (`app_id` = `conversation_<conversation id>`). */
export interface IInboxFileHandle {
	app_id: string;
	path: string;
	bytes: number;
	sha256: string;
	mime: string;
}

export interface IInboxAttachmentPart {
	kind: "attachment";
	/** `a1`, `a2` ... numbered in order within the message. */
	id: string;
	name: string;
	/** A stored file; absent on a linked file. */
	handle?: IInboxFileHandle;
	/** A linked file: http(s), may expire. */
	url?: string;
	/** The type of a linked file. */
	mime?: string;
	/**
	 * A file of an end-to-end encrypted conversation: `handle` names its ciphertext, this says how
	 * to open it. It exists only inside the sealed message.
	 */
	sealed?: IInboxSealedFile;
}

export interface IInboxSealedFile {
	/** `xchacha20poly1305-stream-64k-v1`. */
	alg: string;
	/** The file's key, base64url. */
	key: string;
	/** The file's own type; `handle.mime` is the ciphertext's. */
	mime: string;
	/** The file's own size; `handle.bytes` is the ciphertext's. */
	bytes: number;
}

/** A finished conversation file upload: its `handle` goes into an attachment part. */
export interface IConversationFileReady extends IInboxFileHandle {
	file_id: string;
	name: string;
}

export interface IConversationFileDownload {
	/** A signed link for a few minutes, or a `blob:` URL for a file on this computer. */
	url: string;
	expires_at?: string;
	name: string;
	mime: string;
	bytes: number;
}

export const CONVERSATION_FILE_APP_PREFIX = "conversation_";
/** The most one conversation file can hold, like the 1:1 chat's uploads. */
export const MAX_CONVERSATION_FILE_BYTES = 35 * 1024 * 1024;

export interface IInboxMessageBody {
	text: string;
	usage_stats?: IChatUsageStat[];
	reply_to?: string;
	/** Attachments, numbered `a1`, `a2` ... in order. */
	parts?: IInboxAttachmentPart[];
	/** People and bots the message names; a mentioned bot in the room wakes. */
	mentions?: string[];
	/** Set by a chat that answers this message itself; hosts leave it alone meanwhile. */
	interactive?: { client_run_id: string };
	approval?: IInboxApproval;
	approval_response?: IInboxApprovalResponse;
	user_context_request?: IUserContextRequest;
	user_context_response?: IUserContextResponse;
	user_context_revoke?: IUserContextRevoke;
	/** The note a bot reply wrote or changed; the text is only the owner's to read. */
	note?: IBotNoteRef;
}

export type UserContextField =
	| "location"
	| "timezone"
	| "locale"
	| "preferences"
	| "availability"
	| "free_busy"
	| "current_context"
	| "device";

export interface IUserContextRequest {
	status_change?: {
		status: import("../../lib/availability").AvailabilityStatus;
		duration_minutes: number;
	};
	id: string;
	bot_id: string;
	subject_id: string;
	conversation_id: string;
	purpose: string;
	fields: UserContextField[];
	precision: "approximate" | "precise";
	audience: "direct" | "group";
	audience_member_ids: string[];
	created_at: number;
	expires_at: number;
}

export interface IUserContextResponse {
	request_id: string;
	bot_id: string;
	subject_id: string;
	conversation_id: string;
	decision: "once" | "session" | "deny";
	captured_at: number;
	expires_at: number;
	source: { session_id: string; platform: string };
	values: Partial<Record<UserContextField, Record<string, unknown>>>;
	unavailable: Partial<Record<UserContextField, string>>;
}

export interface IUserContextRevoke {
	request_id: string;
	bot_id: string;
	subject_id: string;
	conversation_id: string;
	revoked_at: number;
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
	/** Set on a `secret_request` card: the owner types the value instead of allowing a call. */
	secret?: IInboxApprovalSecret;
	/** Set on a `room_invite` card: who the bot wants to bring into the room, and why. */
	invite?: IInboxApprovalInvite;
	configuration_changes?: Record<string, unknown>;
	/** A standing grant was set aside because the turn read web pages first. */
	after_web_content?: boolean;
	/** Two or more calls of one model round; the top-level fields then describe the batch. */
	items?: IInboxApprovalItem[];
}

/** One call of a batch card; `scope` is comma-joined, empty when the call is never remembered. */
export interface IInboxApprovalItem {
	args_hash: string;
	tool: string;
	summary: string;
	scope: string;
	/** Identical calls the item stands for; 1 when absent. */
	calls?: number;
	after_web_content?: boolean;
	secret?: IInboxApprovalSecret;
	invite?: IInboxApprovalInvite;
}

/** The exact username or email address the bot was given, and its reason. */
export interface IInboxApprovalInvite {
	person: string;
	reason: string;
}

export interface IInboxApprovalSecret {
	name: string;
	description: string;
	replace?: boolean;
}

/** `decision` covers every item that `items` does not list. */
export interface IInboxApprovalResponse {
	approval_id: string;
	decision: BotApprovalDecision;
	items?: IBotApprovalItemDecision[];
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

/** A call line: `started` carries the call id as its `message_id`; `ended` names it. */
export type IInboxCallBody =
	| { op: "started" }
	| { op: "ended"; call: string; duration_ms: number };

export interface IInboxMemberBody {
	op: "join" | "leave";
	principal_id: string;
	by: string;
}

export interface IConversationMember {
	/** A user sub or a bot id. */
	principal_id: string;
	/** The participant's public display name, when available. */
	name?: string;
	kind: InboxPrincipalKind;
	added_by: string | null;
	joined_at: string;
	/** A bot's room limits; only its owner sees them. */
	limits?: IBotRoomLimits | null;
}

/** What a bot may do and remember in one room, within its owner's permissions. */
export interface IBotRoomLimits {
	/** Most permissions (role bits) the bot holds in the room; omitted adds no limit. */
	mask?: number | null;
	/** Apps the bot may use in the room; omitted adds no limit. */
	apps?: string[] | null;
	/** Further scopes per app id. */
	scopes?: Record<string, IBotScopes>;
	/** The bot answers only when a message names it or replies to it; it doesn't read along. */
	mentioned_only?: boolean;
	/** Recall the owner's private-chat memory in this group. Defaults to false. */
	use_owner_memory?: boolean;
	/** Recall knowledge shared by this bot's other groups. Defaults to false. */
	use_shared_memory?: boolean;
	/** Make this group's saved knowledge available outside it. Defaults to false. */
	share_memory?: boolean;
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
	/** A group's picture: a signed link, or a data URL for a conversation on this device. */
	image_url?: string | null;
	encryption: ConversationEncryption;
	created_by: string;
	created_at: string;
	last_activity_at: string;
	/** The reader's latest history deletion; changes make clients discard older cached history. */
	history_cleared_at?: string | null;
	/** Active members only. */
	members: IConversationMember[];
	/** Invitations nobody accepted yet; they read and receive nothing. */
	pending?: IPendingMember[];
	unread_count: number;
	/** You muted it: messages arrive and count as unread, but send no push. */
	muted?: boolean;
	/** When you pinned it to the top of your list; null when it isn't pinned. */
	pinned_at?: string | null;
	/** Newest `message` entry in the reader's inbox. */
	last_entry: IInboxEntry | null;
	/** Lives only on this device, between you and local bots, until it moves to an account. */
	local?: boolean;
}

export interface IConversationPinState {
	conversation_id: string;
	pinned_at: string | null;
}

/** A message pinned to a conversation for every member. */
export interface IPinnedMessage {
	message_id: string;
	pinned_by: string;
	pinned_at: string;
	/** The reader's copy; null when they joined after it was sent. */
	entry: IInboxEntry | null;
	/** The sender's newest edit of it in the reader's copy. */
	latest_edit: IInboxEntry | null;
}

/** The most messages one conversation keeps pinned. */
export const MAX_PINNED_MESSAGES = 50;

/** A message you starred, kept for you across conversations. */
export interface IStarredMessage {
	conversation_id: string;
	message_id: string;
	starred_at: string;
	entry: IInboxEntry;
	/** The sender's newest edit of it in your copy. */
	latest_edit: IInboxEntry | null;
	/** Opaque; pass it back as `before`. */
	cursor: string;
}

export interface IStarredMessagesPage {
	/** Newest star first. */
	stars: IStarredMessage[];
	next_before: string | null;
}

export interface IStarredMessagesQuery {
	/** Only this conversation's stars. */
	conversationId?: string;
	before?: string;
	/** 1 to 200; 50 when omitted. */
	limit?: number;
}

export interface ICreateConversationRequest {
	kind: ConversationKind;
	title?: string | null;
	/** Everyone except the caller. A direct conversation takes exactly one. */
	members: string[];
	encryption?: ConversationEncryption;
	/** Settings for your own bots when creating a group. */
	bot_limits?: Record<string, IBotRoomLimits>;
}

export interface ISendInboxEntryRequest {
	/** Chosen by the sender; sending the same id again returns the first result. */
	message_id: string;
	kind: Exclude<InboxEntryKind, "member" | "trigger" | "activity" | "room">;
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

/** A person or bot blocked by the signed-in account. */
export interface IBlockedPrincipal {
	principal_id: string;
	kind: InboxPrincipalKind;
	created_at: string;
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
	/** `direct` is a message request, `group` a room invitation. */
	conversation_kind?: ConversationKind;
	/** A request's messages held for you: how many, and the newest few. */
	held_count?: number;
	held?: IHeldMessage[];
}

/** A message a request holds until you answer it. */
export interface IHeldMessage {
	message_id: string;
	created_at: string;
	envelope: IInboxPlainEnvelope;
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

export interface IConversationLink {
	id: string;
	conversation_id: string;
	token: string;
	name: string | null;
	/** User sub of the member who made it. */
	created_by: string;
	/** -1 is unlimited. */
	max_uses: number;
	count_joined: number;
	expires_at: string | null;
	created_at: string;
}

export interface ICreateConversationLinkRequest {
	name?: string | null;
	/** Omit or 0 or less for unlimited. */
	max_uses?: number | null;
	/** Omit or 0 or less to never expire. */
	expires_in_hours?: number | null;
}

export interface IJoinedConversation {
	conversation: IConversation;
	/** False when you were already a member. */
	joined: boolean;
}

export type ConversationTurnStatus = "queued" | "running" | "waiting";

/** A bot turn on an account conversation that has not finished. */
export interface IConversationTurn {
	id: string;
	bot_id: string;
	status: ConversationTurnStatus;
	trigger_message_id: string;
	/** What the bot says it is doing right now. */
	status_text?: string | null;
	created_at: string;
	updated_at: string;
}

export interface IInboxState {
	/** Newest activity first. `as` reads one of your bots' inboxes. */
	listConversations(as?: string): Promise<IConversation[]>;
	getConversation(conversationId: string, as?: string): Promise<IConversation>;
	/** Clears your visible history and removes the conversation from your list. In groups, you and your bots also leave. */
	deleteConversation(conversationId: string): Promise<void>;
	/** A direct conversation that already exists is returned instead of a new one. */
	createConversation(
		request: ICreateConversationRequest,
	): Promise<IConversation>;
	renameConversation(
		conversationId: string,
		title: string,
	): Promise<IConversation>;
	/** Sets a group's picture from an image data URL of at most 512 KiB; `null` removes it. */
	setConversationImage(
		conversationId: string,
		image: string | null,
	): Promise<IConversation>;
	addMembers(conversationId: string, members: string[]): Promise<IConversation>;
	removeMember(conversationId: string, principalId: string): Promise<void>;
	listEntries(
		conversationId: string,
		query?: IInboxEntriesQuery,
	): Promise<IInboxEntriesPage>;
	/** Pending invitations for you and your bots, newest first. */
	listInvitations(): Promise<IInvitation[]>;
	listBlocks(): Promise<IBlockedPrincipal[]>;
	/** Prevents direct contact and invitations; blocking a person also covers their bots. */
	blockPrincipal(principalId: string): Promise<void>;
	unblockPrincipal(principalId: string): Promise<void>;
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
	/** Declines a message request and hides every other invitation from the same person. */
	blockInvitation(invitation: IInvitation): Promise<void>;
	setConversationMuted(conversationId: string, muted: boolean): Promise<void>;
	/** Pinned conversations list first, newest pin first. */
	setConversationPinned(
		conversationId: string,
		pinned: boolean,
	): Promise<IConversationPinState>;
	/** The conversation's pinned messages, newest pin first; `as` reads as one of your bots. */
	listPins(conversationId: string, as?: string): Promise<IPinnedMessage[]>;
	/** Pins a message for every member; pinning it again changes nothing. */
	pinMessage(
		conversationId: string,
		messageId: string,
	): Promise<IPinnedMessage>;
	unpinMessage(conversationId: string, messageId: string): Promise<void>;
	/** Stars a message for you only; starring it again changes nothing. */
	starMessage(
		conversationId: string,
		messageId: string,
	): Promise<IStarredMessage>;
	unstarMessage(conversationId: string, messageId: string): Promise<void>;
	listStars(query?: IStarredMessagesQuery): Promise<IStarredMessagesPage>;
	cancelInvitation(conversationId: string, principalId: string): Promise<void>;
	createConversationLink(
		conversationId: string,
		request: ICreateConversationLinkRequest,
	): Promise<IConversationLink>;
	/** Newest first. */
	listConversationLinks(conversationId: string): Promise<IConversationLink[]>;
	revokeConversationLink(conversationId: string, linkId: string): Promise<void>;
	/** Joining a room you are already in answers `joined: false`. */
	joinConversationLink(token: string): Promise<IJoinedConversation>;
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
	 * Puts a file into a conversation you are an active member of and returns the handle for the
	 * attachment part. Fails with `ATTACHMENTS_ENCRYPTED_UNSUPPORTED` in an end-to-end encrypted
	 * conversation.
	 */
	uploadConversationFile(
		conversationId: string,
		file: File,
	): Promise<IConversationFileReady>;
	/** A short-lived link to a conversation file; `as` reads as one of your bots. */
	conversationFileDownload(
		conversationId: string,
		handle: Pick<IInboxFileHandle, "path" | "mime">,
		as?: string,
	): Promise<IConversationFileDownload>;
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
	/**
	 * A secret's value sealed on this device to the bot's published keys, ready for
	 * `upsertBotSecret`'s `sealed`; only the bot's host opens it. Set where this app can seal.
	 */
	sealBotSecret?(
		botId: string,
		name: string,
		value: string,
	): Promise<IInboxSealedEnvelope>;
	/** Unfinished bot turns in an account conversation, oldest first; local conversations have none. */
	listConversationTurns?(conversationId: string): Promise<IConversationTurn[]>;
	/** Signaling credentials for the conversation's peer room; account conversations only. */
	getConversationRealtimeAccess?(
		conversationId: string,
	): Promise<IRealtimeAccess>;
}

function timeOf(value: string | null | undefined): number {
	const time = value ? Date.parse(value) : Number.NaN;
	return Number.isNaN(time) ? 0 : time;
}

/** Pinned conversations first, newest pin first; then newest activity first. */
export function compareConversations(
	a: Pick<IConversation, "id" | "pinned_at" | "last_activity_at">,
	b: Pick<IConversation, "id" | "pinned_at" | "last_activity_at">,
): number {
	const pinned = Number(Boolean(b.pinned_at)) - Number(Boolean(a.pinned_at));
	if (pinned !== 0) return pinned;
	const order =
		a.pinned_at && b.pinned_at
			? timeOf(b.pinned_at) - timeOf(a.pinned_at)
			: timeOf(b.last_activity_at) - timeOf(a.last_activity_at);
	if (order !== 0) return order;
	if (a.id === b.id) return 0;
	return a.id < b.id ? 1 : -1;
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
