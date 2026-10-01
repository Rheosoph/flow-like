import type { CatalogNode, CatalogScores } from "./NodeReference";

export type CatalogSummary = Pick<
	CatalogNode,
	| "slug"
	| "name"
	| "friendlyName"
	| "description"
	| "category"
	| "categorySlug"
	| "icon"
	| "inputCount"
	| "outputCount"
> & { scores?: Pick<CatalogScores, "security"> };

// Directory cards never need pin schemas, OAuth scopes, or node documentation.
// Keep that metadata on the individual reference page, outside hydration props.
export function toCatalogSummary(node: CatalogNode): CatalogSummary {
	return {
		slug: node.slug,
		name: node.name,
		friendlyName: node.friendlyName,
		description: node.description,
		category: node.category,
		categorySlug: node.categorySlug,
		icon: node.icon,
		inputCount: node.inputCount,
		outputCount: node.outputCount,
		scores: node.scores ? { security: node.scores.security } : undefined,
	};
}

export const DIRECTORY_PAGE_SIZE = 48;
