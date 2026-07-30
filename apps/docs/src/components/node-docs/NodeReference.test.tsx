import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import {
	type CatalogNode,
	type CatalogScores,
	NodeCatalogOverview,
	NodeCategoryOverview,
	NodeReference,
} from "./NodeReference";

function node(scores?: CatalogScores): CatalogNode {
	return {
		slug: "nodes/testing/example",
		packageName: "flow-like-catalog",
		name: "example",
		friendlyName: "Example",
		description: "Example node",
		category: "Testing",
		categoryPath: ["Testing"],
		categorySlug: "testing",
		scores,
		pins: [],
		inputCount: 0,
		outputCount: 0,
		flags: [],
		oauthProviders: [],
		requiredOauthScopes: {},
		permissions: [],
	};
}

describe("NodeReference catalog scores", () => {
	test("renders raw high-impact scores consistently", () => {
		const markup = renderToStaticMarkup(
			<NodeReference
				node={node({
					security: 8,
					privacy: 7,
					performance: 6,
					governance: 5,
					reliability: 4,
					cost: 3,
				})}
			/>,
		);

		expect(markup).toContain("<strong>8/10</strong>");
		expect(markup).toContain("<span>8/10</span><small>High</small>");
		expect(markup).not.toContain("Security exposure</span><strong>2/10");
	});

	test("marks nodes without score metadata as unrated", () => {
		const reference = renderToStaticMarkup(<NodeReference node={node()} />);
		expect(reference).toContain("<strong>Unrated</strong>");
		expect(reference).toContain(
			"No score metadata has been set for this node yet.",
		);

		const catalog = renderToStaticMarkup(
			<NodeCatalogOverview nodes={[node()]} categories={[]} />,
		);
		expect(catalog).toContain("Security unrated");
		expect(catalog).toContain('<option value="unrated">Unrated</option>');
	});

	test("leaves the page-level heading to the docs layout", () => {
		const reference = renderToStaticMarkup(<NodeReference node={node()} />);
		const category = renderToStaticMarkup(
			<NodeCategoryOverview
				category="Testing"
				label="Testing"
				nodes={[node()]}
			/>,
		);
		const catalog = renderToStaticMarkup(
			<NodeCatalogOverview nodes={[node()]} categories={[]} />,
		);

		expect(reference).not.toContain("<h1");
		expect(category).not.toContain("<h1");
		expect(catalog).not.toContain("<h1");
	});
});
