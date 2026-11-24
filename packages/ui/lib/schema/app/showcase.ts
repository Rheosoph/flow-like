import { asArray, isRecord } from "../../response-shape";

/**
 * Store page Showcase. Hand-maintained mirror of `packages/api/src/routes/app/showcase/types.rs`.
 *
 * Settings are owner-controlled and start off. The public payload is filtered on the server:
 * an absent field means the owner hid it, never that it failed to load.
 */

export interface IShowcaseToggle {
	enabled: boolean;
}

export interface IShowcaseDataSettings {
	enabled: boolean;
	/** Table names the owner ticked. New tables stay hidden until ticked. */
	tables: string[];
	row_counts: boolean;
	schema: boolean;
	/** Up to five rows per table. Personal columns are always masked. */
	samples: boolean;
	ontology: boolean;
	sources: boolean;
}

export interface IShowcaseTriggerSettings {
	enabled: boolean;
	/** Event ids the owner ticked. New events stay hidden until ticked. */
	events: string[];
	/** HTTP method + path and cron schedules. */
	technical: boolean;
}

export interface IShowcasePageSettings {
	enabled: boolean;
	/** A wireframe built from each page's component tree, without any text or data. */
	previews: boolean;
}

export interface IShowcaseHoodSettings {
	enabled: boolean;
	runtime: boolean;
	models: boolean;
	packages: boolean;
	accounts: boolean;
}

export interface IShowcaseActivitySettings {
	enabled: boolean;
	runs: boolean;
	versions: boolean;
}

export interface IAppShowcaseSettings {
	map: boolean;
	data: IShowcaseDataSettings;
	triggers: IShowcaseTriggerSettings;
	pages: IShowcasePageSettings;
	widgets: IShowcaseToggle;
	templates: IShowcaseToggle;
	flows: IShowcaseToggle;
	hood: IShowcaseHoodSettings;
	activity: IShowcaseActivitySettings;
}

export type IShowcaseColumnFamily =
	| "text"
	| "number"
	| "time"
	| "bool"
	| "vector"
	| "struct"
	| "geo"
	| "binary"
	| "other";

export interface IShowcaseColumn {
	name: string;
	display_name?: string | null;
	description?: string | null;
	data_type: string;
	family: IShowcaseColumnFamily;
	nullable: boolean;
	vector_size?: number | null;
	/** Marked PII or classified confidential or higher. Always null in samples. */
	personal: boolean;
}

export interface IShowcaseIndex {
	name: string;
	index_type: string;
	columns: string[];
}

export interface IShowcaseSample {
	/** Same order as `IShowcaseTable.columns`. Personal columns hold null. */
	rows: unknown[][];
}

export interface IShowcaseObjectType {
	name: string;
	color?: string | null;
	icon?: string | null;
}

export interface IShowcaseTable {
	name: string;
	/** Present when row counts are shown. */
	rows?: number | null;
	/** Present when the schema is shown. */
	columns?: IShowcaseColumn[] | null;
	indexes?: IShowcaseIndex[] | null;
	/** Present when sample rows are shown and the table has rows. */
	sample?: IShowcaseSample | null;
	/** Present when object types are shown and the table backs one. */
	object_type?: IShowcaseObjectType | null;
}

export interface IShowcaseObjectLink {
	/** Table names on both ends; both are always shown tables. */
	from_table: string;
	to_table: string;
	label: string;
}

export interface IShowcaseSource {
	id: string;
	name: string;
	alias: string;
	driver: string;
}

export interface IShowcaseEventTechnical {
	method?: string | null;
	path?: string | null;
	cron?: string | null;
	timezone?: string | null;
}

export interface IShowcaseEvent {
	id: string;
	name: string;
	description?: string | null;
	event_type: string;
	/** Present when flows are shown. */
	board_id?: string | null;
	page_id?: string | null;
	/** Shown tables this event's flow opens, also while flows are hidden. */
	tables?: string[] | null;
	/** Shown pages of this event's flow, also while flows are hidden. */
	pages?: string[] | null;
	/** Present when `technical` is on. */
	technical?: IShowcaseEventTechnical | null;
}

export interface IShowcaseScores {
	security: number;
	privacy: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
}

export interface IShowcaseFlow {
	id: string;
	name: string;
	description: string;
	node_count: number;
	scores?: IShowcaseScores | null;
	/** Shown tables this flow opens by name. */
	tables: string[];
	/** Shown pages that belong to this flow. */
	pages: string[];
}

/** A page's component tree reduced to component types. No text, data or styling. */
export interface IShowcaseOutlineNode {
	/** A2UI component type, e.g. `column`, `table`, `chart`. */
	t: string;
	c?: IShowcaseOutlineNode[];
}

export interface IShowcasePage {
	id: string;
	name: string;
	description?: string | null;
	route?: string | null;
	/** Present when flows are shown. */
	board_id?: string | null;
	/** Present when previews are shown. */
	outline?: IShowcaseOutlineNode | null;
	component_count?: number | null;
}

export interface IShowcaseWidget {
	id: string;
	name: string;
	description?: string | null;
	thumbnail?: string | null;
}

export interface IShowcaseTemplate {
	id: string;
	name: string;
	description?: string | null;
}

export interface IShowcaseModel {
	id: string;
	name: string;
	/** Bit type, e.g. `Llm`, `Embedding`, `Vlm`. */
	kind: string;
}

export interface IShowcasePackage {
	id: string;
	name: string;
	version: string;
	/** `NodePermission` names the package declares. */
	permissions: string[];
}

export interface IShowcaseOAuth {
	provider: string;
	scopes: string[];
}

export interface IShowcaseHood {
	/** `Any`, `Local` or `Remote`; present when the run location is shown. */
	execution_mode?: string | null;
	/** Some node only runs on the member's device. */
	requires_local?: boolean | null;
	models?: IShowcaseModel[] | null;
	packages?: IShowcasePackage[] | null;
	oauth?: IShowcaseOAuth[] | null;
	refuse_bots: boolean;
	allow_forking: boolean;
}

export interface IShowcaseRunDay {
	/** `YYYY-MM-DD` (UTC). */
	day: string;
	runs: number;
}

export interface IShowcaseVersion {
	version: string;
	/** RFC 3339. */
	date?: string | null;
	notes?: string | null;
}

export interface IShowcaseActivity {
	/** Last 30 days, oldest first, zero-filled. Present when run counts are shown. */
	runs?: IShowcaseRunDay[] | null;
	versions?: IShowcaseVersion[] | null;
}

export interface IAppShowcase {
	app_id: string;
	map: boolean;
	tables: IShowcaseTable[];
	links: IShowcaseObjectLink[];
	sources: IShowcaseSource[];
	events: IShowcaseEvent[];
	flows: IShowcaseFlow[];
	pages: IShowcasePage[];
	widgets: IShowcaseWidget[];
	templates: IShowcaseTemplate[];
	hood?: IShowcaseHood | null;
	activity?: IShowcaseActivity | null;
}

export function emptyShowcaseSettings(): IAppShowcaseSettings {
	return {
		map: false,
		data: {
			enabled: false,
			tables: [],
			row_counts: false,
			schema: false,
			samples: false,
			ontology: false,
			sources: false,
		},
		triggers: { enabled: false, events: [], technical: false },
		pages: { enabled: false, previews: false },
		widgets: { enabled: false },
		templates: { enabled: false },
		flows: { enabled: false },
		hood: {
			enabled: false,
			runtime: false,
			models: false,
			packages: false,
			accounts: false,
		},
		activity: { enabled: false, runs: false, versions: false },
	};
}

export function emptyShowcase(appId: string): IAppShowcase {
	return {
		app_id: appId,
		map: false,
		tables: [],
		links: [],
		sources: [],
		events: [],
		flows: [],
		pages: [],
		widgets: [],
		templates: [],
		hood: null,
		activity: null,
	};
}

const SETTING_GROUPS = [
	"data",
	"triggers",
	"pages",
	"widgets",
	"templates",
	"flows",
	"hood",
	"activity",
] as const;

/** Every field present: a partial or older response is laid over the all-off defaults. */
export function mergeShowcaseSettings(
	response: unknown,
	fallback: IAppShowcaseSettings = emptyShowcaseSettings(),
): IAppShowcaseSettings {
	const base = emptyShowcaseSettings();
	if (!isRecord(response)) return fallback;
	const merged: Record<string, unknown> = { ...base };
	if (typeof response.map === "boolean") merged.map = response.map;
	for (const group of SETTING_GROUPS) {
		const incoming = response[group];
		merged[group] = isRecord(incoming)
			? { ...(base[group] as object), ...incoming }
			: base[group];
	}
	const result = merged as unknown as IAppShowcaseSettings;
	result.data.tables = asArray(result.data.tables);
	result.triggers.events = asArray(result.triggers.events);
	return result;
}

/** Lists are always arrays so the store page can map them without guards. */
export function normalizeShowcase(
	appId: string,
	response: unknown,
): IAppShowcase {
	const base = emptyShowcase(appId);
	if (!isRecord(response)) return base;
	const showcase = { ...base, ...response } as IAppShowcase;
	showcase.app_id = appId;
	showcase.tables = asArray(showcase.tables);
	showcase.links = asArray(showcase.links);
	showcase.sources = asArray(showcase.sources);
	showcase.events = asArray(showcase.events);
	showcase.flows = asArray(showcase.flows);
	showcase.pages = asArray(showcase.pages);
	showcase.widgets = asArray(showcase.widgets);
	showcase.templates = asArray(showcase.templates);
	return showcase;
}
