import type { IEventExecutionMode, IExecutionMode } from "../../lib/schema";

export interface IStorageItemActionResult {
	prefix: string;
	url?: string;
	error?: string;
}

export interface IBackendRole {
	id: string;
	app_id: string;
	name: string;
	description: string;
	permissions: bigint;
	attributes?: string[];
	updated_at: string;
	created_at: string;
}

export interface IInviteLink {
	id: string;
	app_id: string;
	token: string;
	count_joined: number;
	name: string;
	max_uses: number;
	created_at: string;
	updated_at: string;
}

export interface IJoinRequest {
	id: string;
	user_id: string;
	app_id: string;
	comment: string;
	created_at: string;
	updated_at: string;
}

export interface IMember {
	id: string;
	user_id: string;
	app_id: string;
	role_id: string;
	joined_via?: string;
	created_at: string;
	updated_at: string;
}

export type IAppConnectionStatus = "PENDING" | "ACTIVE";

export interface IAppConnection {
	id: string;
	source_app_id: string;
	target_app_id: string;
	status: IAppConnectionStatus;
	role_id?: string | null;
	role_name?: string | null;
	/** Raw permission bits granted by the connection role. */
	role_permissions?: number | null;
	comment?: string | null;
	requested_by_user_id?: string | null;
	approved_by_user_id?: string | null;
	app_name?: string | null;
	app_description?: string | null;
	/** Presigned icon URL of the other app. */
	app_icon?: string | null;
	created_at: number;
	updated_at: number;
}

export interface IAppConnectionsResponse {
	incoming: IAppConnection[];
	outgoing: IAppConnection[];
}

export interface IAccessibleApp {
	app_id: string;
	name?: string | null;
	description?: string | null;
}

export interface IRemoteEvent {
	id: string;
	name: string;
	description?: string | null;
	event_type: string;
}

export interface IRemoteRestRoute {
	method: string;
	path: string;
	params: string[];
}

export interface IRemoteRestFile {
	path: string;
	directory: boolean;
	content_type?: string | null;
}

export interface IRemoteMcpTool {
	name: string;
	description?: string | null;
	input_schema?: Record<string, unknown> | null;
}

export interface IRemoteMcpResource {
	uri: string;
	name?: string | null;
	description?: string | null;
	mime_type?: string | null;
}

export interface IRemoteEventDetail {
	id: string;
	name: string;
	description?: string | null;
	event_type: string;
	rest_routes: IRemoteRestRoute[];
	rest_files: IRemoteRestFile[];
	mcp_tools: IRemoteMcpTool[];
	mcp_resources: IRemoteMcpResource[];
}

export interface IProcessNote {
	id: string;
	author_user_id?: string | null;
	content: string;
	created_at: number;
	updated_at: number;
}

export interface IAppContentStats {
	events: number;
	pages: number;
	templates: number;
	widgets: number;
}

export interface IProcessGraphNode {
	id: string;
	name?: string | null;
	description?: string | null;
	icon?: string | null;
	/** Presigned banner/thumbnail URL (null for masked apps). */
	banner?: string | null;
	unknown: boolean;
	is_current: boolean;
	can_annotate: boolean;
	notes: IProcessNote[];
	/** Descriptive tags from the app's metadata (empty for masked apps). */
	tags: string[];
	/** Primary category, e.g. "Productivity" (null for masked apps). */
	category?: string | null;
	/** External website URL from the app's metadata. */
	website?: string | null;
	/** Documentation URL from the app's metadata. */
	docs_url?: string | null;
	/** Summary of what the app contains (null for masked apps). */
	content?: IAppContentStats | null;
}

export interface IProcessGraphEdge {
	source: string;
	target: string;
	status: IAppConnectionStatus;
	role_name?: string | null;
	/** Raw permission bits granted to the source app by the connection role. */
	role_permissions?: number | null;
}

export interface IProcessFlow {
	path: string[];
	run_count: number;
	/** How many of those runs failed/timed-out/were cancelled. */
	failed_count: number;
	/** Mean wall-clock duration of completed runs, in milliseconds. */
	avg_duration_ms?: number | null;
	last_run_at: number;
	/** Event executed on the terminal app (null when that app is masked). */
	event_name?: string | null;
	event_type?: string | null;
}

export interface IProcessGraphResponse {
	nodes: IProcessGraphNode[];
	edges: IProcessGraphEdge[];
	flows: IProcessFlow[];
}

/** An end-to-end process case: a causal execution tree across apps/events. */
export interface IProcessCase {
	case_id: string;
	root_app_id: string;
	root_event_name?: string | null;
	root_event_type?: string | null;
	apps: string[];
	run_count: number;
	failed_count: number;
	correlation_keys?: Record<string, string> | null;
	/** "Completed" | "Running" | "Failed" */
	status: string;
	started_at?: number | null;
	last_activity_at: number;
	duration_ms?: number | null;
}

export interface IProcessCasesResponse {
	cases: IProcessCase[];
}

/** One run inside a case's causal tree, with timing for the waterfall. */
export interface IProcessCaseRun {
	run_id: string;
	app_id: string;
	parent_run_id?: string | null;
	depth: number;
	/** PENDING | RUNNING | COMPLETED | FAILED | CANCELLED | TIMEOUT */
	status: string;
	event_name?: string | null;
	event_type?: string | null;
	started_at: number;
	completed_at?: number | null;
	updated_at: number;
	duration_ms?: number | null;
}

export interface IProcessCaseDetailResponse {
	runs: IProcessCaseRun[];
}

export interface IInvite {
	id: string;
	user_id: string;
	app_id: string;
	name: string;
	description?: string;
	message?: string;
	by_member_id: string;
	created_at: string;
	updated_at: string;
}

export interface IUserLookup {
	id: string;
	email?: string;
	username?: string;
	preferred_username?: string;
	name?: string;
	avatar_url?: string;
	additional_info?: string;
	description?: string;
	created_at: string;
}

export interface INotificationsOverview {
	invites_count: number;
	notifications_count: number;
	unread_count: number;
}

export type NotificationType = "WORKFLOW" | "SYSTEM";

export interface INotification {
	id: string;
	user_id: string;
	app_id?: string;
	title: string;
	description?: string;
	icon?: string;
	link?: string;
	notification_type: NotificationType;
	read: boolean;
	source_run_id?: string;
	source_node_id?: string;
	created_at: string;
	read_at?: string;
}

export interface INotificationEvent {
	title: string;
	description?: string;
	icon?: string;
	link?: string;
	show_desktop: boolean;
	// Optional metadata for persisting workflow notifications via API
	event_id?: string;
	target_user_sub?: string;
	source_run_id?: string;
	source_node_id?: string;
}

/** A runtime-configured variable that needs a value before execution */
export interface IRuntimeVariable {
	id: string;
	name: string;
	description?: string;
	data_type: string;
	value_type: string;
	secret: boolean;
	schema?: string;
}

/** OAuth provider requirement */
export interface IOAuthRequirement {
	provider_id: string;
	scopes: string[];
}

/** Response from pre-run analysis for boards */
export interface IPrerunBoardResponse {
	runtime_variables: IRuntimeVariable[];
	oauth_requirements: IOAuthRequirement[];
	requires_local_execution: boolean;
	execution_mode: IExecutionMode;
	/** Whether user can execute locally (has ReadBoards permission). If false, must execute on server */
	can_execute_locally: boolean;
	/** Whether the board contains any WASM (external) nodes */
	has_wasm_nodes?: boolean;
	/** package_id values of all WASM nodes present in the board */
	wasm_package_ids?: string[];
	/** Per-package deduplicated permissions declared by WASM nodes */
	wasm_package_permissions?: Record<string, string[]>;
	/**
	 * Stable hash over the board-derived fields. Frontends may cache the
	 * response and revalidate in the background; a changed signature
	 * signals the underlying board has shifted.
	 * Optional only because legacy backends may not emit it.
	 */
	signature?: string;
}

/** Response from pre-run analysis for events */
export interface IPrerunEventResponse {
	board_id: string;
	runtime_variables: IRuntimeVariable[];
	oauth_requirements: IOAuthRequirement[];
	requires_local_execution: boolean;
	execution_mode: IExecutionMode;
	/** Event's own execution mode — where this specific event runs. */
	event_execution_mode?: IEventExecutionMode;
	/** Whether user can execute locally (has ReadBoards permission). If false, must execute on server */
	can_execute_locally: boolean;
	/** Whether the board contains any WASM (external) nodes */
	has_wasm_nodes?: boolean;
	/** package_id values of all WASM nodes present in the board */
	wasm_package_ids?: string[];
	/** Per-package deduplicated permissions declared by WASM nodes */
	wasm_package_permissions?: Record<string, string[]>;
	/** See {@link IPrerunBoardResponse.signature}. */
	signature?: string;
}
