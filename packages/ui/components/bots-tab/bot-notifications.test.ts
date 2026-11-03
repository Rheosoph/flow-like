import { describe, expect, test } from "bun:test";
import {
	botNotificationsMuted,
	withNotificationsMuted,
} from "./bot-notifications";

describe("a bot's notification switch", () => {
	test("reads only a true mute flag", () => {
		expect(botNotificationsMuted({ appearance: { kind: "pip" } })).toBe(false);
		expect(
			botNotificationsMuted({
				appearance: { kind: "pip", mute_notifications: true },
			}),
		).toBe(true);
		expect(
			botNotificationsMuted({
				appearance: { kind: "pip", mute_notifications: false },
			}),
		).toBe(false);
	});

	test("muting keeps the look and unmuting leaves no flag in the saved JSON", () => {
		const look = { kind: "pip", dark: true };
		const muted = withNotificationsMuted(look, true);
		expect(muted).toEqual({
			kind: "pip",
			dark: true,
			mute_notifications: true,
		});
		const back = JSON.parse(
			JSON.stringify(withNotificationsMuted(muted, false)),
		);
		expect(back).toEqual(look);
	});
});
