import type { IBot } from "@flow-like/flow-like-ui";
import { describe, expect, test } from "vitest";
import { mergeBots } from "../local-bots/merge";

function bot(
	id: string,
	system: IBot["system"],
	updatedAt = "2026-11-02T09:00:00Z",
	local = false,
): IBot {
	return {
		id,
		system,
		created_at: "2026-11-02T09:00:00Z",
		updated_at: updatedAt,
		local,
	} as unknown as IBot;
}

const accountGeneral = bot("bot_general", "general");
const accountFlux = bot("bot_flux", "flux");

describe("merging the account's bots with this computer's", () => {
	test("this computer's untouched Assistant and Flux give way to the account's own", () => {
		const merged = mergeBots(
			[accountGeneral, accountFlux],
			[
				bot("bot_local_general", "general", undefined, true),
				bot("bot_local_flux", "flux", undefined, true),
				bot("bot_scout", null, undefined, true),
			],
		);
		expect(merged.map((item) => item.id)).toEqual([
			"bot_general",
			"bot_flux",
			"bot_scout",
		]);
	});

	test("a built-in edited or chatted with here stays until it moves", () => {
		const edited = bot(
			"bot_local_general",
			"general",
			"2026-11-02T10:00:00Z",
			true,
		);
		const merged = mergeBots([accountGeneral, accountFlux], [edited]);
		expect(merged.map((item) => item.id)).toEqual([
			"bot_general",
			"bot_flux",
			"bot_local_general",
		]);
	});

	test("without the account's own, this computer's built-in still shows", () => {
		const localFlux = bot("bot_local_flux", "flux", undefined, true);
		expect(mergeBots([accountGeneral], [localFlux])).toEqual([
			accountGeneral,
			localFlux,
		]);
	});
});
