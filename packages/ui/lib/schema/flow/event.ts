export interface IEvent {
	active: boolean;
	board_id: string;
	board_version?: number[] | null;
	canary?: null | ICanaryEvent;
	/**
	 * Canary/shadow targets. Read through the core `variant_set()`: when this is
	 * empty a legacy `canary` still counts as a single Live variant.
	 */
	variants?: IEventVariant[];
	config: number[];
	created_at: ISystemTime;
	description: string;
	/** A2UI: default page to render for this event (page-target events). */
	default_page_id?: string | null;
	/** URL route path that maps to this event (for example, "/" or "/dashboard"). */
	route?: string | null;
	/** Whether this event is the app's default event/route. */
	is_default?: boolean;
	event_type: string;
	event_version: number[];
	/**
	 * Where this event runs. Events are always exactly one of Local or Remote
	 * (never Hybrid). Defaults to "Local" when unset for backward compatibility.
	 */
	execution_mode?: IEventExecutionMode;
	/**
	 * Public HTTP surface for `rest`/`mcp` events. `PUBLIC` (default) serves the
	 * event on its own inbound router with the configured auth; `INTERNAL`
	 * restricts it to connected apps via the app-connection proxy.
	 */
	exposure?: IEventExposure;
	/**
	 * Process-mining case-key mappings: business key name → dot-path into the
	 * invocation payload (e.g. { order_id: "order.id" }). Extracted on every
	 * run so process cases group by business object automatically.
	 */
	correlation_mappings?: { [key: string]: string } | null;
	id: string;
	/** Input pins copied from the node */
	inputs?: IEventInput[];
	name: string;
	node_id: string;
	notes?: IReleaseNotes | null;
	priority: number;
	updated_at: ISystemTime;
	variables: { [key: string]: IVariable };
	[property: string]: any;
}

export enum IEventExecutionMode {
	Local = "Local",
	Remote = "Remote",
}

export enum IEventExposure {
	Public = "PUBLIC",
	Internal = "INTERNAL",
}

/** Simplified input pin metadata for events */
export interface IEventInput {
	id: string;
	name: string;
	friendly_name: string;
	description: string;
	data_type: string;
	value_type: string;
	schema?: string | null;
	default_value?: number[] | null;
	index: number;
}

export interface ICanaryEvent {
	board_id: string;
	board_version?: number[] | null;
	created_at: ISystemTime;
	node_id: string;
	updated_at: ISystemTime;
	variables: { [key: string]: IVariable };
	weight: number;
	[property: string]: any;
}

/**
 * How a variant handles its share of traffic: `Live` replaces the primary
 * target for `weight` of triggers, `Shadow` additionally runs the variant for
 * `sample_rate` of triggers and discards the result.
 *
 * Serde externally-tagged enum: exactly one of `Live` / `Shadow` is set.
 */
export type IEventVariantMode =
	| { Live: { weight: number } }
	| { Shadow: { sample_rate: number } };

/**
 * An alternate dispatch target for an event: a canary (`Live`) or shadow
 * deployment of another board/version/node. Read through the core
 * `variant_set()`, which falls back to the legacy `canary` field.
 */
export interface IEventVariant {
	/** Unique per event, `[a-z0-9-]`; "canary" is the conventional name. */
	name: string;
	board_id: string;
	/**
	 * `null` floats on latest (the etag is bound at dispatch); the u32::MAX
	 * sentinel is reserved and refused at upsert.
	 */
	board_version?: number[] | null;
	node_id: string;
	/** Merged over the event's variables per key, variant wins. */
	variables: { [key: string]: IVariable };
	/** Page-event variants only: repoints what a matched route renders. */
	default_page_id?: string | null;
	mode: IEventVariantMode;
	created_at: ISystemTime;
	updated_at: ISystemTime;
}

export interface ISystemTime {
	nanos_since_epoch: number;
	secs_since_epoch: number;
	[property: string]: any;
}

export interface IVariable {
	category?: null | string;
	data_type: IVariableType;
	default_value?: number[] | null;
	description?: null | string;
	editable: boolean;
	exposed: boolean;
	hash?: number | null;
	id: string;
	name: string;
	schema?: null | string;
	secret: boolean;
	value_type: IValueType;
	[property: string]: any;
}

export enum IVariableType {
	Boolean = "Boolean",
	Byte = "Byte",
	Date = "Date",
	Execution = "Execution",
	Float = "Float",
	Generic = "Generic",
	Integer = "Integer",
	PathBuf = "PathBuf",
	String = "String",
	Struct = "Struct",
}

export enum IValueType {
	Array = "Array",
	HashMap = "HashMap",
	HashSet = "HashSet",
	Normal = "Normal",
}

export interface IReleaseNotes {
	NOTES?: string;
	URL?: string;
}
