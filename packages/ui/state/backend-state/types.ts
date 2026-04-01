import type { IExecutionMode } from "../../lib/schema";

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
}

/** Response from pre-run analysis for events */
export interface IPrerunEventResponse {
	board_id: string;
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
}
