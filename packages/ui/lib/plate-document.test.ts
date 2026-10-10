import { describe, expect, test } from "bun:test";
import {
	parsePlateDocument,
	replacePlateDocumentChildren,
	serializePlateDocument,
	type PlateDocument,
} from "./plate-document";
import { plainTextFromRichContent } from "./plate-text";

const nodes = [{ type: "p", children: [{ text: "Article text" }] }];
const document: PlateDocument = {
	version: 1,
	documentId: "article-1",
	children: nodes,
	users: { reporter: { id: "reporter", name: "Reporter" } },
	discussions: [
		{
			id: "review-1",
			userId: "reporter",
			createdAt: "2026-10-09T12:00:00Z",
			isResolved: true,
			comments: [
				{
					id: "reply-1",
					discussionId: "review-1",
					userId: "reporter",
					createdAt: "2026-10-09T12:00:00Z",
					isEdited: false,
					contentRich: [
						{ type: "p", children: [{ text: "Check the source" }] },
					],
				},
			],
		},
	],
};

describe("Plate document storage", () => {
	test("keeps legacy node arrays readable and byte compatible", () => {
		const legacy = `plate_json::${JSON.stringify(nodes)}`;
		expect(serializePlateDocument(parsePlateDocument(legacy)!)).toBe(legacy);
	});
	test("retains review bodies, identity and resolved state through content transforms", () => {
		const content = serializePlateDocument(document);
		const updated = replacePlateDocumentChildren(content, [
			{ type: "p", children: [{ text: "Revised" }] },
		]);
		const restored = parsePlateDocument(updated)!;
		expect(restored.discussions).toEqual(document.discussions);
		expect(restored.users).toEqual(document.users);
		expect(restored.documentId).toBe("article-1");
		expect(plainTextFromRichContent(updated)).toBe("Revised");
		expect(plainTextFromRichContent(updated)).not.toContain("Check the source");
	});
	test("rejects unsupported versions and malformed envelopes", () => {
		expect(
			parsePlateDocument('plate_json::{"version":99,"children":[]}'),
		).toBeUndefined();
		expect(
			parsePlateDocument(
				'plate_json::{"version":1,"children":{},"discussions":[]}',
			),
		).toBeUndefined();
		expect(
			parsePlateDocument(
				'plate_json::{"version":1,"children":[],"discussions":{}}',
			),
		).toBeUndefined();
		expect(parsePlateDocument("plain markdown")).toBeUndefined();
	});
	test("rejects malformed review entries before rendering discussion controls", () => {
		for (const discussions of [
			[null],
			[{ ...document.discussions[0], comments: [null] }],
			[{ ...document.discussions[0], createdAt: "invalid date" }],
			[
				{
					...document.discussions[0],
					comments: [
						{ ...document.discussions[0]!.comments[0], contentRich: [null] },
					],
				},
			],
			[document.discussions[0], document.discussions[0]],
		]) {
			expect(
				parsePlateDocument(
					`plate_json::${JSON.stringify({ ...document, discussions })}`,
				),
			).toBeUndefined();
		}
		expect(
			parsePlateDocument(
				`plate_json::${JSON.stringify({ ...document, users: { reporter: null } })}`,
			),
		).toBeUndefined();
	});
});
