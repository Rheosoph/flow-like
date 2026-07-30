/**
 * Frontend types for store-wide template discovery. Templates carry no
 * visibility of their own — access is inherited from the owning app — so both
 * of these surfaces are gated on the owning app being publicly visible.
 *
 * Kept loose (plain `string` over exhaustive unions) so future server additions
 * surface as unknown values rather than crashing older clients.
 */

import type { IMetadata } from "../bit/bit";
import type { IAppCategory } from "./app-search-query";

export interface ITemplateSearchQuery {
	query: string;
	language?: string;
	limit?: number;
	offset?: number;
	category?: IAppCategory;
	tag?: string;
	/** Only templates whose owning app allows forking. */
	forkable_only?: boolean;
}

/** One search hit: template metadata plus enough of the owning app to attribute it. */
export interface ITemplateSearchHit {
	app_id: string;
	template_id: string;
	version?: string;
	metadata?: IMetadata;
	app_name?: string;
	app_allow_forking: boolean;
	app_price: number;
	rating_sum: number;
	rating_count: number;
}

/**
 * A template's SHAPE, not its contents. This is deliberately the only template
 * detail a non-member of the owning app can read — counts and node type names,
 * never pin values, variable defaults or the graph itself.
 */
export interface ITemplatePreview {
	app_id: string;
	template_id: string;
	node_count: number;
	layer_count: number;
	variable_count: number;
	node_types: string[];
	node_types_truncated: boolean;
	has_entry_event: boolean;
}
