/**
 * Frontend types mirroring the API's `forking` group. These intentionally
 * stay loose (no exhaustive enum exhaustion) so future server additions
 * surface as `string` rather than `never` and don't crash clients on
 * unknown variants.
 */

import type { IAppCategory, IAppExecutionMode } from "./app";

/** Where the caller wants the fork to land. */
export type IForkPreviewTarget = "online" | "offline";

/** How the project database travels with a fork. */
export type IForkDatabaseMode = "none" | "schema_only" | "with_data";

/**
 * Owner-defined description of what a fork of an app contains. The person
 * forking gets no choice — this is read-only everywhere except the source
 * app's own settings. An app that never configured one reports the
 * permissive default (everything on, `with_data`).
 */
export interface IForkPolicy {
	flows: boolean;
	files: boolean;
	databases: IForkDatabaseMode;
	/** No-op offline — an offline bundle ships no role rows at all. */
	roles: boolean;
	widgets: boolean;
	templates: boolean;
}

/** Bytes + object count for one fork category. */
export interface IForkCategorySize {
	bytes: number;
	objects: number;
}

/**
 * Per-category size of the source app. Database objects contribute bytes
 * but are deliberately left out of the object counts.
 */
export interface IForkSizeBreakdown {
	/** Copied regardless of policy: manifest, events, pages, metadata, media. */
	always: IForkCategorySize;
	flows: IForkCategorySize;
	files: IForkCategorySize;
	databases: IForkCategorySize;
	widgets: IForkCategorySize;
	templates: IForkCategorySize;
}

/** One source table's Arrow schema, for a schema-only database fork. */
export interface IForkTableSchema {
	table: string;
	/** serde-serialized `arrow_schema::Schema`. */
	schema: unknown;
}

/** Project-level fork settings (GET /apps/{app_id}/settings/forking). */
export interface IForkSettings {
	allow_forking: boolean;
	fork_policy: IForkPolicy;
}

/** A single replaceable / re-auth-required token site on the source. */
export type IRemoteTokenSite =
	| { HttpAuthToken: { event_id: string } }
	| { Pat: { event_id: string; sink_id: string } }
	| { OAuth: { event_id: string; sink_id: string } };

export interface IForkPreviewResponse {
	source_app_id: string;
	total_size_bytes: number;
	total_object_count: number;
	max_size_bytes: number;
	max_file_count: number;
	/** Whether the fork fits both caps *after* the owner's policy is applied. */
	within_limits: boolean;
	/** The source owner's policy. Display-only for the forker. */
	fork_policy: IForkPolicy;
	size_breakdown: IForkSizeBreakdown;
	/** Bytes actually copied once the policy is applied. */
	selected_size_bytes: number;
	/** Objects actually copied once the policy is applied. */
	selected_object_count: number;
	requires_token: boolean;
	remote_token_sites: IRemoteTokenSite[];
	allow_forking: boolean;
	user_can_fork: boolean;
	disallow_reason: string;
}

export interface IMetaBlob {
	relative_path: string;
	/** Base64-encoded bytes. Most entries are compressed app artifacts
	 * (proto for boards/events/templates/metadata, JSON for widgets/pages);
	 * `media/...` entries are raw bytes and should be written to local
	 * content storage. */
	data_b64: string;
}

/**
 * Source → destination id translation produced by a fork.
 *
 * `boards` is the one an agent orchestrating a multi-part build needs: a plan's
 * part targets name boards in the SOURCE app, and a fork allocates fresh ids, so
 * every target must be retargeted through this map before it is dispatched.
 * The node/pin maps run to thousands of entries and are of no use to a caller
 * working at board granularity.
 */
export interface IForkIdMap {
	source_app_id?: string;
	app_id?: string;
	boards?: Record<string, string>;
	events?: Record<string, string>;
	pages?: Record<string, string>;
	widgets?: Record<string, string>;
	templates?: Record<string, string>;
	[key: string]: unknown;
}

export interface IForkReport {
	id_map: IForkIdMap;
	skipped: { kind: string; source_id: string; reason: string }[];
	warnings: string[];
	bytes_copied: number;
	objects_copied: number;
}

export interface IBeginOfflineForkResponse {
	new_app_id: string;
	fork_session_id: string;
	/**
	 * Remapped + secret-stripped inline artifacts (manifest, boards,
	 * events, widgets, templates, pages, and DB-backed metadata
	 * files, plus app metadata media). Each entry's `data_b64` is the
	 * exact bytes that would have been written to disk on a destination
	 * — the desktop decodes and writes it under
	 * `apps/{new_app_id}/{relative_path}`.
	 */
	meta_blobs: IMetaBlob[];
	/**
	 * Bucket-relative prefix of the **source** content store (e.g.
	 * `apps/{src_app_id}`). The desktop pulls metadata/, upload/,
	 * storage/ from there directly, translating
	 * `metadata/{widgets|templates|pages}/{src_id}/...` segments
	 * client-side via `id_map` before writing locally.
	 */
	source_content_prefix: string;
	/** Scoped read credentials for `source_content_prefix`. Opaque
	 * blob the desktop hands to the Tauri `apply_fork_bundle` command. */
	shared_credentials: unknown;
	expires_at?: string | null;
	remote_token_sites: IRemoteTokenSite[];
	report: IForkReport;
	/** Files in the source content prefix flagged as suspicious — only
	 * populated when the deployment has a meta/content store-split
	 * misconfiguration. Show as a warning. */
	content_store_leaks?: string[];
	/** The source owner's policy — display only. */
	fork_policy: IForkPolicy;
	/** Prefixes under `source_content_prefix` the desktop must not mirror. */
	content_exclude_prefixes: string[];
	/** Tables to create empty locally for a schema-only database fork. */
	db_table_schemas: IForkTableSchema[];
}

export interface IOnlineForkResponse {
	new_app_id: string;
	report: IForkReport;
}

export interface IForkBundleSummary {
	total_size_bytes: number;
	total_object_count: number;
}

export interface IBeginOnlineForkBody {
	source_app_id?: string | null;
	summary: IForkBundleSummary;
	language?: string;
}

export interface IBeginOnlineForkResponse {
	new_app_id: string;
	fork_session_id: string;
	upload_path: string;
	shared_credentials: unknown;
	expiration?: string | null;
}

export interface IFinalizeOnlineForkBody {
	visibility?: "private" | "prototype";
	app_settings?: IFinalizeOnlineForkAppSettings;
}

export interface IFinalizeOnlineForkAppSettings {
	changelog?: null | string;
	primary_category?: IAppCategory | null;
	secondary_category?: IAppCategory | null;
	price?: null | number;
	version?: null | string;
	execution_mode: IAppExecutionMode;
}

export interface IFinalizeOnlineForkResponse {
	app_id: string;
	total_size_bytes: number;
	total_object_count: number;
	visibility: string;
	status: string;
}

export interface IOnlineForkBody {
	remote_event_token?: string;
	language?: string;
}

export interface IBeginOfflineForkBody {
	remote_event_token?: string;
	language?: string;
}

/** Helper — does this site need a fresh OAuth re-auth (true) or is a
 * single PAT-style token enough (false)? Mirrors
 * `RemoteTokenSite::is_token_replaceable()` on the server. */
export function isTokenReplaceable(site: IRemoteTokenSite): boolean {
	return "HttpAuthToken" in site || "Pat" in site;
}

/** Convenience extractor for the event id any RemoteTokenSite refers to,
 * regardless of which variant it is. */
export function siteEventId(site: IRemoteTokenSite): string {
	if ("HttpAuthToken" in site) return site.HttpAuthToken.event_id;
	if ("Pat" in site) return site.Pat.event_id;
	return site.OAuth.event_id;
}
