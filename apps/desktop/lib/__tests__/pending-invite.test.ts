import { describe, expect, test } from "bun:test";
import { parsePendingInvite, pendingInviteUrl } from "../pending-invite";

const now = 1_000_000;

describe("pending invite", () => {
	test("reads a value stored before kind existed as an app invite", () => {
		const invite = parsePendingInvite(
			JSON.stringify({ appId: "a", token: "t", ts: now }),
			now,
		);
		expect(invite).toEqual({ kind: "app", appId: "a", token: "t", ts: now });
	});

	test("reads room invites", () => {
		const invite = parsePendingInvite(
			JSON.stringify({ kind: "room", token: "r/1", ts: now }),
			now,
		);
		expect(invite).toEqual({ kind: "room", token: "r/1", ts: now });
		expect(pendingInviteUrl(invite!)).toBe("/join?room=r%2F1");
	});

	test("rejects expired and malformed values", () => {
		const day = 24 * 60 * 60 * 1000;
		expect(
			parsePendingInvite(
				JSON.stringify({ kind: "room", token: "r", ts: now }),
				now + day + 1,
			),
		).toBeNull();
		expect(parsePendingInvite(JSON.stringify({ ts: now }), now)).toBeNull();
		expect(
			parsePendingInvite(JSON.stringify({ token: "t", ts: now }), now),
		).toBeNull();
	});

	test("builds the app join url", () => {
		expect(
			pendingInviteUrl({ kind: "app", appId: "a b", token: "t", ts: 0 }),
		).toBe("/join?appId=a%20b&token=t");
	});
});
