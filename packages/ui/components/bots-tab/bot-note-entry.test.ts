import { describe, expect, test } from "bun:test";
import type { IBotNote } from "../../state/backend-state/bot-state";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import { isNoteConflict, noteRefOf, noteVersionView } from "./bot-note-entry";

function entry(note: unknown): IInboxEntry {
	return {
		envelope: { enc: "none", body: { text: "hi", note } },
	} as unknown as IInboxEntry;
}

const note: IBotNote = {
	key: "plan",
	title: "Plan",
	status: "Drafting",
	body: "v3 body",
	state: "open",
	version: 3,
	sensitivity: { pii: false },
	updated_by: "bot",
	created_at: 1,
	updated_at: 3,
	history: [
		{
			version: 2,
			title: "Plan",
			body: "v2 body",
			state: "open",
			updated_by: "bot",
			updated_at: 2,
		},
	],
};

describe("noteRefOf", () => {
	test("reads a valid reference", () => {
		expect(
			noteRefOf(entry({ key: "plan", version: 2, title: "Plan", status: "x" })),
		).toEqual({ key: "plan", version: 2, title: "Plan", status: "x" });
		expect(
			noteRefOf(entry({ key: "a", version: 1, title: "T", closed: true })),
		).toEqual({ key: "a", version: 1, title: "T", closed: true });
	});

	test("rejects malformed shapes", () => {
		expect(noteRefOf(entry(undefined))).toBeUndefined();
		expect(noteRefOf(entry("plan"))).toBeUndefined();
		expect(
			noteRefOf(entry({ key: "", version: 1, title: "T" })),
		).toBeUndefined();
		expect(
			noteRefOf(entry({ key: "a", version: 0, title: "T" })),
		).toBeUndefined();
		expect(noteRefOf(entry({ key: "a", version: 1 }))).toBeUndefined();
		expect(
			noteRefOf({ envelope: { enc: "x", body: {} } } as unknown as IInboxEntry),
		).toBeUndefined();
	});
});

describe("noteVersionView", () => {
	test("current version", () => {
		const view = noteVersionView(note, 3);
		expect(view.updatedSince).toBe(false);
		expect(view.version.body).toBe("v3 body");
	});

	test("version from history", () => {
		const view = noteVersionView(note, 2);
		expect(view.updatedSince).toBe(true);
		expect(view.version.body).toBe("v2 body");
	});

	test("missing version falls back to current, flagged", () => {
		const view = noteVersionView(note, 1);
		expect(view.updatedSince).toBe(true);
		expect(view.version.version).toBe(3);
	});
});

describe("isNoteConflict", () => {
	test("detects a 409", () => {
		expect(isNoteConflict(Object.assign(new Error("x"), { status: 409 }))).toBe(
			true,
		);
		expect(isNoteConflict(new Error("version conflict"))).toBe(true);
		expect(isNoteConflict(new Error("boom"))).toBe(false);
	});
});
