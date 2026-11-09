import type { IInboxEntry } from "../../state/backend-state/inbox-state";

export interface IRealtimeIceServer {
	urls: string | string[];
	username?: string;
	credential?: string;
}

export interface IRealtimeAccess {
	jwt: string;
	encryption_key: string;
	key_id: string;
	ice_servers?: IRealtimeIceServer[];
	/** Unix timestamp in seconds. */
	ice_expires_at?: number;
}

export interface IJwk {
	kty: string;
	kid?: string;
	alg?: string;
	use?: string;
	// EC (P-256)
	crv?: string;
	x?: string;
	y?: string;
	// RSA (optional fields, not used currently)
	n?: string;
	e?: string;
}

export interface IJwks {
	keys: IJwk[];
}

export const REALTIME_PROTOCOL_VERSION = 1;
export const MAX_REALTIME_RUNS = 16;
export const MAX_REALTIME_CONVERSATIONS = 8;
export const MAX_REALTIME_CONVERSATION_ENTRIES = 50;
export const MAX_REALTIME_EVENTS = 100;
export const MAX_REALTIME_RESPONSE_BYTES = 4 * 1024 * 1024;
export const MAX_REALTIME_TIMEOUT_MS = 20_000;
/** Transient: keep the subscription and its cursor, retry with backoff. */
export const REALTIME_ERROR_UNAVAILABLE = "unavailable";
/** The run does not exist or this account cannot read it. */
export const REALTIME_ERROR_NOT_FOUND = "not_found";

const TERMINAL_RUN_STATUSES = new Set([
	"completed",
	"failed",
	"cancelled",
	"canceled",
	"timeout",
	"timed_out",
]);

export function isTerminalRunStatus(status: string): boolean {
	return TERMINAL_RUN_STATUSES.has(status.toLowerCase());
}

export interface RealtimeRunSubscription {
	/** Owned runs require current log access or a signed capability for this run. */
	run_id: string;
	poll_token?: string;
	after_sequence?: number;
	revision?: string;
}

export interface RealtimeConversationSubscription {
	conversation_id: string;
	/** An `IInboxEntry.cursor`; omitted only while no entries are held. */
	after?: string;
	/** Read as one of my bots. */
	as?: string;
}

export interface RealtimePollRequest {
	notifications?: { revision?: string };
	runs: RealtimeRunSubscription[];
	conversations?: RealtimeConversationSubscription[];
	timeout_ms: number;
}

export interface RealtimeNotificationSnapshot {
	revision: string;
	overview: {
		invites_count: number;
		notifications_count: number;
		unread_count: number;
	};
}

export interface RealtimeRunEvent {
	sequence: number;
	event_id?: string;
	event_type: string;
	payload: unknown;
	created_at: string;
}

export interface RealtimeRunSnapshot {
	run_id: string;
	revision: string;
	status: string;
	progress: number;
	current_step?: string | null;
	error?: string | null;
	events: RealtimeRunEvent[];
	after_sequence: number;
	has_more: boolean;
}

export interface RealtimeConversationUpdate {
	conversation_id: string;
	as?: string;
	/** Oldest first. */
	entries: IInboxEntry[];
	has_more: boolean;
}

export interface RealtimeConversationError {
	conversation_id: string;
	as?: string;
	code: string;
}

export interface RealtimePollResponse {
	protocol_version: 1;
	notifications?: RealtimeNotificationSnapshot;
	runs: RealtimeRunSnapshot[];
	errors: { run_id: string; code: string }[];
	conversations?: RealtimeConversationUpdate[];
	conversation_errors?: RealtimeConversationError[];
}

export type RealtimeConnectionState =
	| "connecting"
	| "connected"
	| "reconnecting"
	| "paused"
	| "unauthorized"
	| "unsupported"
	| "closed";

export interface RealtimeSubscription {
	notifications: boolean;
	/** Keep notification delivery active while every tab in this account is hidden. */
	backgroundNotifications?: boolean;
	runs: RealtimeRunSubscription[];
	conversations?: RealtimeConversationSubscription[];
}

export interface RealtimeConnectionSession {
	/** Local account routing key. The API independently authenticates every request. */
	scope: string;
	url: string;
	headers: Record<string, string>;
	/** Prefer HTTP when this caller does not need a provider connection. */
	transport?: "configured" | "http";
}

/** Receive-only grants. Execution replies continue to use their separate channel handles. */
export type RealtimeTransport =
	| { type: "http" }
	| { type: "nats"; url: string; subject: string; token: string }
	| {
			type: "aws_mqtt";
			endpoint: string;
			region: string;
			client_id: string;
			topic: string;
			credentials: {
				access_key_id: string;
				secret_access_key: string;
				session_token: string;
				expiration: number;
			};
	  }
	| { type: "azure_web_pubsub"; url: string; group: string }
	| {
			type: "gcp_firebase_rtdb";
			database_url: string;
			api_key: string;
			project_id: string;
			custom_token: string;
			path: string;
	  };

export interface RealtimeConnectResponse {
	protocol_version: 1;
	/** Unix seconds. A replacement grant is needed before this time. */
	expires_at: number;
	/** Seconds until `expires_at` by the server clock. */
	expires_in?: number;
	/** Longest replay `timeout_ms` the API holds. Zero: poll on an interval. */
	max_poll_wait_ms: number;
	transport: RealtimeTransport;
}

export interface RealtimeWakeup {
	protocol_version: 1;
	kind: "invalidate";
	nonce: string;
}

export interface RealtimeActivity {
	online: boolean;
	visible: boolean;
}

/** Where a provider connection attempt stopped; `lost` is a live connection that dropped. */
export type RealtimeProviderStage =
	| "grant"
	| "handshake"
	| "connect"
	| "subscribe"
	| "lost";

export interface RealtimeProviderFailure {
	stage: RealtimeProviderStage;
	message: string;
	elapsedMs: number;
	/** Epoch milliseconds. */
	at: number;
}

/** Whether wakeups arrive over a provider connection or only the replay's own polls deliver. */
export interface RealtimeProviderStatus {
	/** The transport the API granted; unset until it answered. */
	transport?: RealtimeTransport["type"];
	live: boolean;
	/** The newest failure, kept after the connection recovers. */
	failure?: RealtimeProviderFailure;
}

/** What a tab knows about its account's realtime connection. */
export interface RealtimeStatus {
	runner: "shared-worker" | "in-tab";
	state: RealtimeConnectionState;
	provider?: RealtimeProviderStatus;
}

export type RealtimeUpdate =
	| { type: "state"; state: RealtimeConnectionState }
	| { type: "provider"; status: RealtimeProviderStatus }
	| { type: "snapshot"; snapshot: RealtimePollResponse };

export type RealtimeListener = (update: RealtimeUpdate) => void | Promise<void>;

/** Implemented by browser workers and native subscription bridges. */
export interface RealtimeClient {
	setSession(session: RealtimeConnectionSession): void;
	setSubscriptions(subscription: RealtimeSubscription): void;
	setActivity(activity: RealtimeActivity): void;
	subscribe(listener: RealtimeListener): () => void;
	close(): void;
}

export interface RealtimeClientOptions {
	onUnauthorized?: (
		rejectedToken?: string,
	) => Promise<Record<string, string> | undefined>;
}

export type RealtimeCommand =
	| {
			type: "connect";
			id: string;
			session: RealtimeConnectionSession;
			activity: RealtimeActivity;
	  }
	| { type: "session"; id: string; session: RealtimeConnectionSession }
	| {
			type: "subscribe";
			id: string;
			subscription: RealtimeSubscription;
			subscription_version?: number;
	  }
	| { type: "heartbeat"; id: string; activity: RealtimeActivity }
	| { type: "accepted"; id: string; delivery_id: string }
	| {
			type: "authorize";
			id: string;
			request_id: string;
			headers?: Record<string, string>;
			failed?: boolean;
	  }
	| { type: "disconnect"; id: string };

export type RealtimeMessage =
	| { type: "ready" }
	| { type: "expired" }
	| { type: "state"; state: RealtimeConnectionState }
	| { type: "provider"; status: RealtimeProviderStatus }
	| {
			type: "snapshot";
			delivery_id: string;
			snapshot: RealtimePollResponse;
			subscription_version?: number;
	  }
	| { type: "unauthorized"; request_id: string; rejectedToken?: string };

export interface RealtimePort {
	postMessage(message: RealtimeMessage): void;
}
