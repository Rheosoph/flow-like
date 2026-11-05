import { describe, expect, test } from "bun:test";
import { notificationBotId } from "./bot-notification-avatar";

describe("the bot behind a notification", () => {
	test("is named by its chat link", () => {
		expect(notificationBotId("/bots?bot=bot_1")).toBe("bot_1");
		expect(notificationBotId("/bots?bot=bot_1&conversation=c")).toBe("bot_1");
	});

	test("is absent for any other link", () => {
		expect(notificationBotId("/store/packages")).toBeNull();
		expect(notificationBotId("/bots?bot=")).toBeNull();
		expect(notificationBotId(undefined)).toBeNull();
	});
});
