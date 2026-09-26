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

	test("reads quotes saved before and after Plate 53 made them containers", () => {
		expect(
			plainTextFromRichContent(
				'plate_json::[{"type":"blockquote","children":[{"text":"flat "},{"text":"quote","italic":true}]}]',
			),
		).toBe("flat quote");
		expect(
			plainTextFromRichContent(
				'plate_json::[{"type":"blockquote","children":[{"type":"p","children":[{"text":"first"}]},{"type":"p","children":[{"text":""}]},{"type":"blockquote","children":[{"type":"p","children":[{"text":"nested "},{"type":"a","url":"https://x.dev","children":[{"text":"link"}]}]}]}]}]',
			),
		).toBe("first\nnested link");
	});

	test("puts every table cell on its own line", () => {
		expect(
			plainTextFromRichContent(
				'plate_json::[{"type":"table","children":[{"type":"tr","children":[{"type":"td","children":[{"type":"p","children":[{"text":"a"}]}]},{"type":"td","children":[{"type":"p","children":[{"type":"a","url":"https://x.dev","children":[{"text":"b"}]}]}]}]}]}]',
			),
		).toBe("a\nb");
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
