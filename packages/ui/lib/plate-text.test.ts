import { describe, expect, test } from "bun:test";
import { plainTextFromRichContent } from "./plate-text";

describe("plainTextFromRichContent", () => {
	test("reads the text out of a plate document", () => {
		expect(
			plainTextFromRichContent(
				'plate_json::[{"children":[{"text":"Hi there"}],"type":"p","id":"33oAW4iRvc"}]',
			),
		).toBe("Hi there");
	});

	test("joins blocks and flattens nested marks", () => {
		expect(
			plainTextFromRichContent(
				'plate_json::[{"type":"p","children":[{"text":"one "},{"text":"bold","bold":true}]},{"type":"p","children":[{"text":"two"}]}]',
			),
		).toBe("one bold\ntwo");
	});

	test("leaves markdown content untouched", () => {
		expect(plainTextFromRichContent("# Heading\n\nbody")).toBe(
			"# Heading\n\nbody",
		);
	});

	test("returns nothing rather than JSON when the document is unreadable", () => {
		expect(plainTextFromRichContent("plate_json::{not json")).toBe("");
		expect(plainTextFromRichContent("plate_json::[]")).toBe("");
	});
});
