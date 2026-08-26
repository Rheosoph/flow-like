import { describe, expect, it } from "bun:test";
import rehypeFlowbookTables from "./rehype-flowbook-tables.mjs";

function paragraph(value) {
	return {
		type: "element",
		tagName: "p",
		properties: {},
		children: [{ type: "text", value }],
	};
}

describe("rehypeFlowbookTables", () => {
	it("wraps release checks in a supported data-nosnippet host", () => {
		const releaseCheck = {
			type: "element",
			tagName: "blockquote",
			properties: {},
			children: [
				paragraph("Release check: verify this against a named release."),
			],
		};
		const tree = { type: "root", children: [releaseCheck] };

		rehypeFlowbookTables()(tree);

		expect(tree.children[0]).toEqual({
			type: "element",
			tagName: "div",
			properties: { dataNosnippet: "" },
			children: [releaseCheck],
		});
		expect(releaseCheck.properties).toEqual({});
	});

	it("leaves ordinary blockquotes eligible for snippets", () => {
		const quote = {
			type: "element",
			tagName: "blockquote",
			properties: {},
			children: [
				paragraph("A useful quotation for readers and search results."),
			],
		};
		const tree = { type: "root", children: [quote] };

		rehypeFlowbookTables()(tree);

		expect(tree.children).toEqual([quote]);
	});
});
