import { describe, expect, test } from "bun:test";
import {
	NO_ROOM_LIMITS,
	PERMISSION_BITS,
	draftFromLimits,
	limitsFromDraft,
	maskFromPermissions,
	permissionsFromMask,
	toggleApp,
} from "./room-limits-model";

describe("room limits model", () => {
	test("permission bits are distinct powers of two below the role bits", () => {
		const bits = Object.values(PERMISSION_BITS);
		expect(new Set(bits).size).toBe(bits.length);
		for (const bit of bits) expect(bit & (bit - 1)).toBe(0);
		expect(PERMISSION_BITS.read_files).toBe(16);
		expect(PERMISSION_BITS.execute_events).toBe(8192);
	});

	test("a mask round-trips through its permissions", () => {
		const mask = maskFromPermissions(["read_files", "execute_events"]);
		expect(mask).toBe(16 + 8192);
		expect(permissionsFromMask(mask)).toEqual(["read_files", "execute_events"]);
		expect(permissionsFromMask(0)).toEqual([]);
	});

	test("answering only when mentioned is a limit of its own", () => {
		const draft = draftFromLimits({ mentioned_only: true });
		expect(draft.mentionedOnly).toBe(true);
		expect(limitsFromDraft(draft)).toEqual({ mentioned_only: true });
		expect(
			limitsFromDraft({ ...NO_ROOM_LIMITS, mentionedOnly: false }),
		).toBeNull();
	});

	test("no draft limit means no limits at all", () => {
		expect(limitsFromDraft(NO_ROOM_LIMITS)).toBeNull();
		expect(draftFromLimits(null)).toEqual(NO_ROOM_LIMITS);
	});

	test("an empty permission list is a real limit, not no limit", () => {
		const draft = draftFromLimits({ mask: 0 });
		expect(draft.restrictions.permissions).toEqual([]);
		expect(limitsFromDraft(draft)).toEqual({ mask: 0 });
	});

	test("apps and earlier scopes survive an edit", () => {
		const previous = {
			mask: 16,
			apps: ["a"],
			scopes: { a: { files: { prefixes: ["x"], read_only: true } } },
		};
		const draft = draftFromLimits(previous);
		expect(limitsFromDraft(draft, previous)).toEqual(previous);
	});

	test("unticking an app narrows and re-ticking all apps lifts the limit", () => {
		const all = ["a", "b"];
		const narrowed = toggleApp(NO_ROOM_LIMITS, "a", all, false);
		expect(narrowed.apps).toEqual(["b"]);
		expect(toggleApp(narrowed, "a", all, true).apps).toBeNull();
	});
});
