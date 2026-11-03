import { describe, expect, test } from "bun:test";
import {
	type IBot,
	isBotCapabilityAvailable,
} from "../../state/backend-state/bot-state";
import { offersEncryption } from "./bot-encryption";

const bot = (patch: Partial<IBot>) =>
	({ local: false, capabilities: [], ...patch }) as unknown as IBot;

describe("encryption on the Details tab", () => {
	test("is offered to account bots that hold the capability", () => {
		expect(offersEncryption(bot({ capabilities: ["encryption"] }))).toBe(true);
		expect(offersEncryption(bot({ capabilities: ["internet"] }))).toBe(false);
		expect(
			offersEncryption(bot({ local: true, capabilities: ["encryption"] })),
		).toBe(false);
	});

	test("a server bot can't hold the capability, a desktop or device bot can", () => {
		expect(isBotCapabilityAvailable("encryption", "server")).toBe(false);
		expect(isBotCapabilityAvailable("encryption", "desktop")).toBe(true);
		expect(isBotCapabilityAvailable("encryption", "device")).toBe(true);
	});
});
