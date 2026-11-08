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

const isolated = {
	use_owner_memory: false,
	use_shared_memory: false,
	share_memory: false,
};

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
		expect(limitsFromDraft(draft)).toEqual({
			...isolated,
			mentioned_only: true,
		});
		expect(
			limitsFromDraft({ ...NO_ROOM_LIMITS, mentionedOnly: false }),
		).toEqual(isolated);
	});

	test("new and existing groups default to isolated memory", () => {
		expect(limitsFromDraft(NO_ROOM_LIMITS)).toEqual(isolated);
		expect(draftFromLimits(null)).toEqual(NO_ROOM_LIMITS);
	});

	test("an empty permission list is a real limit, not no limit", () => {
		const draft = draftFromLimits({ mask: 0 });
		expect(draft.restrictions.permissions).toEqual([]);
		expect(limitsFromDraft(draft)).toEqual({ ...isolated, mask: 0 });
	});

	test("apps and earlier scopes survive an edit", () => {
		const previous = {
			mask: 16,
			apps: ["a"],
			scopes: { a: { files: { prefixes: ["x"], read_only: true } } },
		};
		const draft = draftFromLimits(previous);
		expect(limitsFromDraft(draft, previous)).toEqual({
			...isolated,
			...previous,
		});
	});

	test("unticking an app narrows and re-ticking all apps lifts the limit", () => {
		const all = ["a", "b"];
		const narrowed = toggleApp(NO_ROOM_LIMITS, "a", all, false);
		expect(narrowed.apps).toEqual(["b"]);
		expect(toggleApp(narrowed, "a", all, true).apps).toBeNull();
	});
});

describe("independent group memory choices", () => {
	test("all eight combinations round-trip without changing other limits", () => {
		for (const use_owner_memory of [false, true]) {
			for (const use_shared_memory of [false, true]) {
				for (const share_memory of [false, true]) {
					const policy = {
						use_owner_memory,
						use_shared_memory,
						share_memory,
						apps: ["sales"],
					};
					expect(limitsFromDraft(draftFromLimits(policy), policy)).toEqual(
						policy,
					);
				}
			}
		}
	});
	test("turning sharing off stores an explicit false while recall stays enabled", () => {
		const previous = {
			use_owner_memory: true,
			use_shared_memory: true,
			share_memory: true,
		};
		const draft = { ...draftFromLimits(previous), shareMemory: false };
		expect(limitsFromDraft(draft, previous)).toEqual({
			...previous,
			share_memory: false,
		});
	});
});
