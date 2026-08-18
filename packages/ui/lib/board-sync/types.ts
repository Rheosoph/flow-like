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

export interface IBoardSyncRequest {
	meta?: string;
	variables?: string;
	comments?: string;
	layers?: Record<string, string>;
	segments?: Record<string, string>;
	/** The client holds this app's node catalog and will rebuild catalog-owned fields. */
	hydrate?: boolean;
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
}

export interface ISyncSegment {
	hash: string;
	nodes: Record<string, ISyncNode>;
}

export interface IBoardSyncResponse {
	manifest: IBoardSyncManifest;
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
