/**
 * Contract of the node-suggestion recommender (the editor's "ghost node").
 *
 * Every fact is keyed by catalog node `name` and pin `name`, never by ids (the editor mints fresh
 * ids on every placement), and carries no pin values, labels or comments. That keeps facts from the
 * user's other projects safe to cache on the device and identical between training and serving.
 */

/** Bump when the fact shape or the extraction semantics change; every cached corpus and model is dropped. */
export const SUGGESTION_FORMAT_VERSION = 1;

/** Exec predecessors of the anchor node, nearest first. */
export const MAX_PRED = 4;
/** Distinct node types feeding the anchor node's data inputs. */
export const MAX_UPS = 8;
/** Distinct node types already on the board, most recent first. */
export const MAX_BAG = 32;

/**
 * A ghost is shown only when its confidence reaches this. Calibrated offline (eval/run-eval.ts):
 * ~70% of shown ghosts are the node that followed, on ~1 in 5 output pins.
 */
export const GHOST_MIN_CONFIDENCE = 0.55;
/** Alternatives a user can cycle through for one anchor pin. */
export const GHOST_ALTERNATIVES = 5;

export type EdgeKind = "exec" | "data";

/** What the models see for one output pin that is about to be extended. */
export interface SuggestionContext {
	kind: EdgeKind;
	/** Node type of the anchor node. */
	src: string;
	/** Name of the anchor output pin. */
	srcPin: string;
	/** `IPin.data_type` of the anchor pin. */
	dataType: string;
	/** `IPin.value_type` of the anchor pin. */
	valueType: string;
	/** JSON-schema `title` of the anchor pin (refs resolved), `""` when it has none. */
	schema: string;
	pred: string[];
	ups: string[];
	bag: string[];
}

/** A training example: the context plus the node that actually followed it. */
export interface TransitionFact extends SuggestionContext {
	dst: string;
	/** Input pin of `dst` the wire landed on. */
	dstPin: string;
}

/**
 * Facts of one board in a compact, index-based form: a board of 300 nodes shares one type table
 * instead of repeating up to `MAX_BAG` names per edge. Expand with `expandTransition`.
 */
export interface BoardFacts {
	/** Distinct node types in approximate construction order (first appearance). */
	types: string[];
	transitions: FactTransition[];
	/** One row per output pin of every non-reroute node — trains the "no follow-up" model. */
	outcomes: FactOutcome[];
	/** Indices into `types` placed on more than one node (self term of the co-occurrence table). */
	repeated?: number[];
}

export interface FactTransition {
	kind: EdgeKind;
	/** Index into `BoardFacts.types`. */
	src: number;
	srcPin: string;
	dataType: string;
	valueType: string;
	schema: string;
	/** Indices into `types`, nearest predecessor first, at most `MAX_PRED`. */
	pred: number[];
	/** Indices into `types`, at most `MAX_UPS`. */
	ups: number[];
	/** The bag is `types.slice(0, bag)` — the types that existed when `src` was placed. */
	bag: number;
	dst: number;
	dstPin: string;
}

export interface FactOutcome {
	kind: EdgeKind;
	src: number;
	srcPin: string;
	connected: boolean;
}

export interface RankedCandidate {
	type: string;
	probability: number;
}

/** A model that scores follow-up node types. Probabilities are over the model's whole vocabulary. */
export interface SuggestionModel {
	rank(context: SuggestionContext, limit: number): RankedCandidate[];
	serialize(): string;
}

export interface RankedSuggestion {
	type: string;
	/** Predicted input pin of `type` the wire lands on; `null` lets the editor pick a compatible one. */
	pin: string | null;
	probability: number;
}

export interface SuggestionResult {
	/** Best first, self-type policy applied, not yet filtered by pin compatibility. */
	candidates: RankedSuggestion[];
	/** Chance the anchor pin stays unconnected in a finished board. */
	pUnconnected: number;
	/** `(1 - pUnconnected) * candidates[0].probability`, `0` without candidates. */
	confidence: number;
}

/** The slice of a catalog `INode` the neural model featurizes; built once per catalog identity. */
export interface CatalogEntry {
	name: string;
	friendlyName: string;
	category: string;
	description: string;
	pins: CatalogEntryPin[];
}

export interface CatalogEntryPin {
	name: string;
	pinType: "Input" | "Output";
	dataType: string;
	valueType: string;
	schema: string;
}
