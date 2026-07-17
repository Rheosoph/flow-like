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
