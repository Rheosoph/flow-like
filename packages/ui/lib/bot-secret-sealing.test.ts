import { describe, expect, test } from "bun:test";
import type { IInboxKey } from "../state/backend-state/inbox-state";
import {
	botSecretRequest,
	canSealBotSecret,
	sealsSecret,
} from "./bot-secret-sealing";
import { INBOX_KEY_ALGORITHM } from "./inbox-e2e";

function key(principal: string, algorithm = INBOX_KEY_ALGORITHM): IInboxKey {
	return {
		principal_id: principal,
		key_id: `ik_${principal}`,
		algorithm,
		public_key: "pk",
		created_at: "2026-11-12T00:00:00Z",
	};
}

const bot = {
	id: "bot_a",
	capabilities: ["encryption" as const],
	local: false,
};

describe("canSealBotSecret", () => {
	test("needs an account bot with encryption and a published key of its own", () => {
		expect(canSealBotSecret(bot, [key("bot_a")])).toBe(true);
		expect(canSealBotSecret(bot, [])).toBe(false);
		expect(canSealBotSecret(bot, undefined)).toBe(false);
		expect(canSealBotSecret(bot, [key("someone_else")])).toBe(false);
		expect(canSealBotSecret(bot, [key("bot_a", "rsa")])).toBe(false);
		expect(canSealBotSecret({ ...bot, capabilities: [] }, [key("bot_a")])).toBe(
			false,
		);
		expect(canSealBotSecret({ ...bot, local: true }, [key("bot_a")])).toBe(
			false,
		);
		expect(canSealBotSecret(undefined, [key("bot_a")])).toBe(false);
	});
});

describe("sealsSecret", () => {
	test("seals only where sealing is available and left on", () => {
		expect(sealsSecret("available", true)).toBe(true);
		expect(sealsSecret("available", false)).toBe(false);
		expect(sealsSecret("unavailable", true)).toBe(false);
		expect(sealsSecret("checking", true)).toBe(false);
		expect(sealsSecret(undefined, true)).toBe(false);
	});
});

describe("botSecretRequest", () => {
	const envelope = {
		v: 1 as const,
		enc: INBOX_KEY_ALGORITHM,
		ct: "Y3Q",
		keys: [],
	};

	test("a sealed request carries the envelope and never the value", async () => {
		const calls: string[][] = [];
		const inbox = {
			sealBotSecret: async (botId: string, name: string, value: string) => {
				calls.push([botId, name, value]);
				return envelope;
			},
		};
		const request = await botSecretRequest(
			inbox,
			"bot_a",
			"API_KEY",
			"sk-123",
			"Weather",
			true,
		);
		expect(calls).toEqual([["bot_a", "API_KEY", "sk-123"]]);
		expect(request).toEqual({ sealed: envelope, description: "Weather" });
		expect(JSON.stringify(request)).not.toContain("sk-123");
	});

	test("a server request carries the value and keeps an omitted description omitted", async () => {
		expect(
			await botSecretRequest(undefined, "bot_a", "K", "v", undefined, false),
		).toEqual({ value: "v" });
		expect(
			await botSecretRequest(undefined, "bot_a", "K", "v", null, false),
		).toEqual({ value: "v", description: null });
	});

	test("sealing without a sealing app fails instead of sending the value", async () => {
		await expect(
			botSecretRequest({}, "bot_a", "API_KEY", "sk-123", null, true),
		).rejects.toThrow("cannot seal");
	});
});
