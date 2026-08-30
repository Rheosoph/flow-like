/**
 * Wire types of `POST /apps/{app}/board/{board}/sync`. Mirrors
 * `packages/core/src/flow/board/sync.rs` — change both together.
 *
 * Every token is opaque: the client stores what the server sent and echoes it back. It never
 * derives one.
 */
import type {
	IComment,
	IExecutionMode,
	IExecutionStage,
	ILayer,
	ILogLevel,
	INodeScores,
	INodeWasm,
	IPinOptions,
	IPinType,
	ISystemTime,
	IValueType,
	IVariable,
	IVariableType,
} from "../schema/flow/board";
import type { IFnRefs } from "../schema/flow/node";

export interface IBoardSyncManifest {
	meta: string;
	variables: string;
	comments: string;
	/** One token per layer definition, keyed by layer id. */
	layers: Record<string, string>;
	segments: Record<string, string>;
}

/**
 * The tokens of the current revision that differ from the request's. Absent parts kept their
 * token; removed layers/segments are on the response's `dropped_*` lists. Applied onto the
 * request's manifest this yields the revision's full manifest.
 */
export interface IBoardSyncManifestDelta {
	meta?: string | null;
	variables?: string | null;
	comments?: string | null;
	layers?: Record<string, string>;
	segments?: Record<string, string>;
}

export interface IBoardSyncRequest {
	meta?: string;
	variables?: string;
	comments?: string;
	layers?: Record<string, string>;
	segments?: Record<string, string>;
	/** The client holds this app's node catalog and will rebuild catalog-owned fields. */
	hydrate?: boolean;
	/**
	 * The client understands segment patches (`ISyncSegment.base`) and manifest deltas. Sent by
	 * every client built from this module; the server never sends either without it.
	 */
	patch?: boolean;
}

export interface IBoardMeta {
	id: string;
	name: string;
	description: string;
	viewport: number[];
	version: number[];
	stage: IExecutionStage;
	log_level: ILogLevel;
	execution_mode: IExecutionMode;
	page_ids: string[];
	hash?: number | null;
	created_at: ISystemTime;
	updated_at: ISystemTime;
}

export interface ISyncPin {
	id: string;
	name: string;
	index: number;
	schema?: string | null;
	connected_to?: string[];
	depends_on?: string[];
	/** Base64-encoded bytes. */
	default_value?: string | null;
	friendly_name?: string | null;
	description?: string | null;
	pin_type?: IPinType | null;
	data_type?: IVariableType | null;
	value_type?: IValueType | null;
	options?: IPinOptions | null;
}

export interface ISyncNode {
	id: string;
	name: string;
	version?: number | null;
	coordinates?: number[] | null;
	layer?: string | null;
	comment?: string | null;
	start?: boolean | null;
	error?: string | null;
	hash?: number | null;
	fn_refs?: IFnRefs | null;
	wasm?: INodeWasm | null;
	/** Users can rename nodes, so this always ships. */
	friendly_name: string;
	pins: Record<string, ISyncPin>;
	/**
	 * Catalog-owned fields were omitted; rebuild them from the catalog entry for `name`. Only
	 * ever `true` when the request asked for hydration.
	 */
	h?: boolean;
	description?: string | null;
	category?: string | null;
	icon?: string | null;
	docs?: string | null;
	scores?: INodeScores | null;
	long_running?: boolean | null;
	event_callback?: boolean | null;
	only_offline?: boolean | null;
	oauth_providers?: string[] | null;
	required_oauth_scopes?: Record<string, string[]> | null;
	namespace?: string | null;
	alias?: string | null;
	receiver?: string | null;
}

/**
 * Without `base`: the segment's complete node set, replacing the held one wholesale. With
 * `base`: a patch onto the held segment revision `base` — `nodes` are upserts, `removed` are ids
 * that no longer exist. `hash` is always the token of the resulting revision.
 */
export interface ISyncSegment {
	hash: string;
	nodes: Record<string, ISyncNode>;
	base?: string | null;
	removed?: string[];
}

export interface IBoardSyncResponse {
	/** Exactly one of `manifest` and `manifest_delta` is set (the delta only under `patch`). */
	manifest?: IBoardSyncManifest | null;
	manifest_delta?: IBoardSyncManifestDelta | null;
	meta?: IBoardMeta | null;
	variables?: Record<string, IVariable> | null;
	comments?: Record<string, IComment> | null;
	/** Changed or new layer definitions, keyed by layer id. */
	layers?: Record<string, ILayer>;
	dropped_layers?: string[];
	/**
	 * Exactly the refs the parts in this response reference. Content-addressed, so they are
	 * upserted into the held table; nothing is ever removed client-side.
	 */
	refs?: Record<string, string>;
	segments?: Record<string, ISyncSegment>;
	dropped_segments?: string[];
}
