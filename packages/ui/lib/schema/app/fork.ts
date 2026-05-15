/**
 * Frontend types mirroring the API's `forking` group. These intentionally
 * stay loose (no exhaustive enum exhaustion) so future server additions
 * surface as `string` rather than `never` and don't crash clients on
 * unknown variants.
 */

/** Where the caller wants the fork to land. */
export type IForkPreviewTarget = "online" | "offline";

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
	within_limits: boolean;
	requires_token: boolean;
	remote_token_sites: IRemoteTokenSite[];
	allow_forking: boolean;
	user_can_fork: boolean;
	disallow_reason: string;
}

export interface IMetaBlob {
	relative_path: string;
	/** Base64-encoded compressed bytes (proto for boards/events/templates,
	 * JSON for widgets/pages). Decode and write to local meta store. */
	data_b64: string;
}

export interface IForkReport {
	id_map: Record<string, unknown>;
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
	 * files). Each entry's `data_b64` is the exact bytes that would
	 * have been written to disk on a server-side destination — the
	 * desktop decodes and writes it under
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
