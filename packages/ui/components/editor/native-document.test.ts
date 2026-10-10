import { expect, test } from "bun:test";
import {
	type PlateDocument,
	serializePlateDocument,
} from "../../lib/plate-document";
import {
	MAX_NATIVE_DOCUMENT_BYTES,
	deserializeNativeDocumentFile,
	serializeNativeDocument,
} from "./native-document";

const document: PlateDocument = {
	version: 1,
	documentId: "article",
	children: [
		{
			type: "p",
			children: [{ text: "Verified report", comment_thread: true }],
		},
		{
			type: "img",
			url: "storage://editor/photo.png",
			license: "Licensed for the original edition",
			rightsExpiresAt: 1,
			children: [{ text: "" }],
		},
	],
	users: { reporter: { id: "reporter", name: "Reporter" } },
	discussions: [
		{
			id: "thread",
			userId: "reporter",
			createdAt: "2026-10-09T12:00:00Z",
			isResolved: true,
			comments: [
				{
					id: "reply",
					discussionId: "thread",
					userId: "reporter",
					createdAt: "2026-10-09T12:00:00Z",
					isEdited: false,
					contentRich: [{ type: "p", children: [{ text: "Source checked" }] }],
				},
			],
		},
	],
};

test("native JSON file round trip preserves content, anchors and complete review history", async () => {
	const exported = serializeNativeDocument(document);
	expect(JSON.parse(exported)).toEqual(document);
	expect(exported.startsWith("plate_json::")).toBe(false);
	const restored = await deserializeNativeDocumentFile(
		new File([exported], "article.plate.json", { type: "application/json" }),
	);
	expect(restored).toEqual(document);
	expect(serializePlateDocument(restored).startsWith("plate_json::")).toBe(
		true,
	);
});

test("native import accepts raw legacy arrays and old prefixed export files", async () => {
	for (const content of [
		JSON.stringify(document.children),
		`plate_json::${JSON.stringify(document.children)}`,
		serializePlateDocument(document),
	]) {
		const restored = await deserializeNativeDocumentFile(
			new File([content], "article.plate.json"),
		);
		expect(restored.children).toEqual(document.children);
	}
});

test("oversized native files are rejected before reading their contents", async () => {
	let read = false;
	await expect(
		deserializeNativeDocumentFile({
			size: MAX_NATIVE_DOCUMENT_BYTES + 1,
			text: async () => {
				read = true;
				return "[]";
			},
		}),
	).rejects.toThrow("20 MiB");
	expect(read).toBe(false);
});

test("malformed native node trees never reach editor transforms", async () => {
	for (const content of [
		"[null]",
		'[{"children":{}}]',
		'{"version":99,"children":[]}',
	]) {
		await expect(
			deserializeNativeDocumentFile(new File([content], "bad.json")),
		).rejects.toThrow();
	}
});
