import { describe, expect, test } from "bun:test";
import { isBannerHidden, parseBannerHidden } from "./local-bots-banner-model";

const HIDDEN_AT = Date.parse("2026-11-18T12:00:00Z");
const hidden = { account: "felix", at: HIDDEN_AT };
const before = { created_at: "2026-11-02T09:30:00Z" };
const after = { created_at: "2026-11-18T12:00:01Z" };

describe("hiding the banner for bots on this computer", () => {
	test("stays hidden for the bots that were here when it was hidden", () => {
		expect(isBannerHidden(hidden, "felix", [before, before])).toBe(true);
	});

	test("comes back once a bot is made here afterwards", () => {
		expect(isBannerHidden(hidden, "felix", [before, after])).toBe(false);
	});

	test("shows for another account and when it was never hidden", () => {
		expect(isBannerHidden(hidden, "someone-else", [before])).toBe(false);
		expect(isBannerHidden(null, "felix", [before])).toBe(false);
	});

	test("a bot with an unreadable creation time doesn't bring it back", () => {
		expect(isBannerHidden(hidden, "felix", [{ created_at: "soon" }])).toBe(
			true,
		);
	});

	test("reads back only what it stored", () => {
		expect(parseBannerHidden(JSON.stringify(hidden))).toEqual(hidden);
		for (const raw of [
			null,
			"",
			"not json",
			"null",
			"[]",
			'{"account":"felix"}',
			'{"account":"felix","at":"yesterday"}',
			'{"account":7,"at":1}',
		]) {
			expect(parseBannerHidden(raw)).toBeNull();
		}
	});
});
