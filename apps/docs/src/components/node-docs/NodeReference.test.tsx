import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import {
	type CatalogNode,
	type CatalogScores,
	NodeCatalogOverview,
	NodeCategoryOverview,
	NodeReference,
} from "./NodeReference";
import { DIRECTORY_PAGE_SIZE, toCatalogSummary } from "./catalog-summary";

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
	test("keeps schema and permission payloads out of directory hydration", () => {
		const full = {
			...node(),
			docs: "Long reference",
			permissions: ["filesystem"],
			pins: [{ schema: "large-schema" }],
		} as CatalogNode;
		const summary = toCatalogSummary(full);
		expect(JSON.stringify(summary)).not.toContain("large-schema");
		expect(summary).not.toHaveProperty("permissions");
		expect(summary).not.toHaveProperty("docs");
		expect(
			renderToStaticMarkup(
				<NodeCatalogOverview nodes={[summary]} categories={[]} />,
			),
		).toContain("Example node");
	});

	test("bounds initial cards while keeping all links available without JavaScript", () => {
		const nodes = Array.from({ length: DIRECTORY_PAGE_SIZE + 2 }, (_, i) => ({
			...toCatalogSummary(node()),
			name: `node-${i}`,
			slug: `nodes/testing/node-${i}`,
		}));
		const html = renderToStaticMarkup(
			<NodeCatalogOverview nodes={nodes} categories={[]} />,
		);
		expect(html.match(/class="node-card"/g)).toHaveLength(DIRECTORY_PAGE_SIZE);
		expect(html).toContain("Show 2 more nodes");
		expect(html).toContain(
			`<a href="/nodes/testing/node-${DIRECTORY_PAGE_SIZE + 1}/">`,
		);
		expect(html).toContain("<noscript>");
	});

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
