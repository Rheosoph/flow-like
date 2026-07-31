import { describe, expect, test } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { TextEditor } from "./text-editor";

describe("TextEditor", () => {
	test("renders complete Plate JSON documents larger than 50,000 characters", () => {
		const content = `plate_json::${JSON.stringify([
			{
				type: "p",
				children: [{ text: `start-${"x".repeat(50_100)}-end-sentinel` }],
			},
		])}`;

		const html = renderToStaticMarkup(
			createElement(TextEditor, { initialContent: content, isMarkdown: true }),
		);

		expect(html).toContain("start-");
		expect(html).toContain("end-sentinel");
		expect(html).not.toContain("plate_json::");
		expect(html).not.toContain("content truncated for performance");
	});
});

describe("TextEditor markdown tables", () => {
	const markdown = [
		"| Category | Nodes | What's inside |",
		"| --- | --- | --- |",
		"| Digital Twin | 6 | Direct DTR access: create/get/delete shells, add submodels |",
		"| Discovery | 4 | Discovery Finder, BPN Discovery, EDC Discovery and bridges |",
	].join("\n");

	const html = renderToStaticMarkup(
		createElement(TextEditor, { initialContent: markdown, isMarkdown: true }),
	);

	test("renders one visible table and keeps the Slate tracking table hidden", () => {
		expect(html.match(/<table/g)?.length).toBe(2);
		expect(html.match(/<table style="display:none"/g)?.length).toBe(1);
	});

	test("does not force descendant tables to display:block", () => {
		expect(html).not.toContain("_table]:block");
	});

	test("offers a way to reveal truncated cell text", () => {
		expect(html).toContain("Click to expand");
		expect(html).toContain(">Wrap<");
		expect(html).toContain(
			"Direct DTR access: create/get/delete shells, add submodels",
		);
	});
});
