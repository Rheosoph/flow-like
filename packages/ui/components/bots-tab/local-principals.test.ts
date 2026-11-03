import { describe, expect, test } from "bun:test";
import { type IBot, LOCAL_OWNER_ID } from "../../state/backend-state/bot-state";
import { localPrincipalResolver } from "./local-principals";

function bot(id: string, local: boolean): IBot {
	return {
		id,
		owner_id: local ? LOCAL_OWNER_ID : "me",
		system: null,
		preset_id: null,
		name: id,
		description: null,
		instructions: null,
		appearance: { kind: "pip" },
		model: { primary: null, fallbacks: [] },
		classification: { classification: "internal", pii: false },
		capabilities: [],
		profile_id: null,
		location: "desktop",
		device_id: null,
		created_at: "2026-11-02T08:00:00Z",
		updated_at: "2026-11-02T08:00:00Z",
		local,
	} as IBot;
}

const inLocalChats = new Set(["bot_local", "bot_moved", "bot_gone"]);
const isLocal = (id: string) => inLocalChats.has(id);

describe("resolving principals of chats on this computer", () => {
	const resolve = localPrincipalResolver(
		[bot("bot_local", true), bot("bot_moved", false)],
		isLocal,
		"Deleted bot",
	);

	test("a local bot comes from the bot list", () => {
		expect(resolve("bot_local")?.name).toBe("bot_local");
		expect(resolve("bot_local")?.bot?.deleted).toBe(false);
	});

	test("a bot that moved to the account is left to the directory, not shown as deleted", () => {
		expect(resolve("bot_moved")).toBeUndefined();
	});

	test("a local bot that is gone reads as deleted once the list has loaded", () => {
		expect(resolve("bot_gone")).toMatchObject({
			name: "Deleted bot",
			bot: { deleted: true },
		});
		expect(
			localPrincipalResolver(undefined, isLocal, "Deleted bot")("bot_gone"),
		).toBeNull();
	});

	test("you and principals outside local chats are left to the directory", () => {
		expect(resolve(LOCAL_OWNER_ID)).toBeUndefined();
		expect(resolve("user-anna")).toBeUndefined();
	});
});
