import { describe, expect, test } from "bun:test";
import type { IInboxAttachmentPart } from "../state/backend-state/inbox-state";
import { conversationOfApp } from "./conversation-files";
import {
	directAttachmentOf,
	storedAttachmentOf,
} from "./inbox-attachment-part";
import { parseWorkspaceFileUrl, workspaceFileUrl } from "./workspace-file-url";

const handle = (app_id: string, path: string) => ({
	app_id,
	path,
	bytes: 10,
	sha256: "abc",
	mime: "application/pdf",
});

describe("workspaceFileUrl", () => {
	test("encodes segments but keeps the separators", () => {
		expect(workspaceFileUrl(handle("app 1", "bot/files/Größe #1.pdf"))).toBe(
			"storage://apps/app%201/app/bot/files/Gr%C3%B6%C3%9Fe%20%231.pdf",
		);
	});

	test("round-trips through the parser", () => {
		const source = handle("app_x", "bot/files/a b/ü#?.pdf");
		expect(parseWorkspaceFileUrl(workspaceFileUrl(source))).toEqual({
			appId: "app_x",
			path: "bot/files/a b/ü#?.pdf",
		});
	});

	test("rejects other urls", () => {
		expect(parseWorkspaceFileUrl("https://a.test/x")).toBeUndefined();
		expect(parseWorkspaceFileUrl("storage://apps/a/user/x")).toBeUndefined();
	});
});

describe("directAttachmentOf", () => {
	const part = (
		extra: Partial<IInboxAttachmentPart>,
	): IInboxAttachmentPart => ({
		kind: "attachment",
		id: "a1",
		name: "x.pdf",
		...extra,
	});

	test("link part becomes a chip with its mime", () => {
		expect(
			directAttachmentOf(
				part({ url: "https://a.test/x.pdf", mime: "application/pdf" }),
			),
		).toEqual({
			url: "https://a.test/x.pdf",
			name: "x.pdf",
			type: "application/pdf",
		});
	});

	test("workspace handle becomes a storage url", () => {
		expect(
			directAttachmentOf(part({ handle: handle("app1", "bot/x.pdf") })),
		).toEqual({
			url: "storage://apps/app1/app/bot/x.pdf",
			name: "x.pdf",
			type: "application/pdf",
			size: 10,
		});
	});

	test("a stored conversation file gets a url the chat signs through the inbox", () => {
		expect(
			storedAttachmentOf(part({ handle: handle("conversation_c1", "f1.pdf") })),
		).toEqual({
			url: "storage://apps/conversation_c1/app/f1.pdf",
			name: "x.pdf",
			type: "application/pdf",
			size: 10,
		});
		expect(conversationOfApp("conversation_c1")).toBe("c1");
		expect(conversationOfApp("app1")).toBeUndefined();
		expect(
			storedAttachmentOf(part({ url: "https://a.test/x.pdf" })),
		).toMatchObject({ url: "https://a.test/x.pdf" });
	});

	test("conversation files and non-http links are left to the caller", () => {
		expect(
			directAttachmentOf(
				part({ handle: handle("conversation_c1", "f/x.pdf") }),
			),
		).toBeUndefined();
		expect(
			directAttachmentOf(part({ url: "javascript:alert(1)" })),
		).toBeUndefined();
	});
});
