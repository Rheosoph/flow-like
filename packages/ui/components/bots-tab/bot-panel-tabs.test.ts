import { describe, expect, test } from "bun:test";
import type {
	BotCapability,
	BotSystemKey,
	IBot,
} from "../../state/backend-state/bot-state";
import {
	botPanelTabs,
	hasBotWorkspace,
	shownBotPanelTab,
	subjectMemoryBotId,
} from "./bot-panel-tabs";

function bot(
	id: string,
	capabilities: BotCapability[],
	overrides: Partial<IBot> = {},
): IBot {
	return { id, system: null, capabilities, ...overrides } as IBot;
}

function row(subject: IBot) {
	const system: BotSystemKey | null = subject.system;
	return { botId: system === "flux" ? null : subject.id, bot: subject };
}

const everything: BotCapability[] = [
	"internet",
	"memory",
	"storage",
	"data_studio",
];

describe("the panel's tabs follow the bot's abilities", () => {
	test("Details always; Files for storage, Data for data_studio, Memory for memory", () => {
		expect(botPanelTabs(row(bot("bot_scout", everything)))).toEqual([
			"details",
			"files",
			"data",
			"memory",
		]);
		expect(botPanelTabs(row(bot("bot_scout", ["internet"])))).toEqual([
			"details",
		]);
		expect(botPanelTabs(row(bot("bot_scout", ["storage"])))).toEqual([
			"details",
			"files",
		]);
		expect(botPanelTabs(row(bot("bot_scout", ["data_studio"])))).toEqual([
			"details",
			"data",
		]);
		expect(botPanelTabs(row(bot("bot_scout", ["memory"])))).toEqual([
			"details",
			"memory",
		]);
	});

	test("Computer for a bot that drives a browser or the computer, before Memory", () => {
		for (const ability of ["browser_use", "computer_use"] as const) {
			expect(botPanelTabs(row(bot("bot_scout", [ability])))).toEqual([
				"details",
				"computer",
			]);
		}
		expect(
			botPanelTabs(
				row(bot("bot_scout", ["browser_use", "computer_use", "memory"])),
			),
		).toEqual(["details", "computer", "memory"]);
		expect(botPanelTabs(row(bot("bot_scout", ["internet"])))).not.toContain(
			"computer",
		);
	});

	test("a bot on this computer keeps its workspace; a built-in kept there has none yet", () => {
		expect(
			botPanelTabs(row(bot("bot_here", everything, { local: true }))),
		).toEqual(["details", "files", "data", "memory"]);
		expect(
			botPanelTabs(
				row(
					bot("bot_general_here", everything, {
						system: "general",
						local: true,
					}),
				),
			),
		).toEqual(["details", "memory"]);
		expect(
			botPanelTabs(row(bot("bot_general", everything, { system: "general" }))),
		).toEqual(["details", "files", "data", "memory"]);
	});

	test("Flux and its copies always remember and have no Files or Data", () => {
		expect(
			botPanelTabs(row(bot("bot_flux", ["flowpilot"], { system: "flux" }))),
		).toEqual(["details", "memory"]);
		expect(
			botPanelTabs(row(bot("bot_flux_old", [], { system: "flux" }))),
		).toEqual(["details", "memory"]);
		expect(
			botPanelTabs(
				row(bot("bot_flux", ["flowpilot", "storage"], { system: "flux" })),
			),
		).toEqual(["details", "memory"]);
		expect(botPanelTabs(row(bot("bot_flux_copy", ["flowpilot"])))).toEqual([
			"details",
			"memory",
		]);
	});

	test("without a row: Flux and the Assistant by their defaults, a bot that's gone with Details only", () => {
		expect(botPanelTabs({ botId: null })).toEqual(["details", "memory"]);
		expect(botPanelTabs({ botId: "general" })).toEqual(["details", "memory"]);
		expect(botPanelTabs({ botId: "bot_gone" })).toEqual(["details"]);
	});
});

describe("the tab on screen", () => {
	const tabs = botPanelTabs(row(bot("bot_scout", ["memory"])));

	test("is the one asked for while the bot has it, else Details", () => {
		expect(shownBotPanelTab("memory", tabs)).toBe("memory");
		expect(shownBotPanelTab("files", tabs)).toBe("details");
		expect(shownBotPanelTab("details", tabs)).toBe("details");
		expect(shownBotPanelTab(null, tabs)).toBeNull();
	});
});

describe("whose memories the Memory tab lists", () => {
	test("Flux's per profile; every other bot's own, the Assistant without a row as `general`", () => {
		expect(
			subjectMemoryBotId(row(bot("bot_flux", [], { system: "flux" }))),
		).toBe(null);
		expect(subjectMemoryBotId({ botId: null })).toBe(null);
		expect(subjectMemoryBotId({ botId: "general" })).toBe("general");
		expect(subjectMemoryBotId(row(bot("bot_flux_copy", ["flowpilot"])))).toBe(
			"bot_flux_copy",
		);
		expect(
			subjectMemoryBotId(row(bot("bot_general", [], { system: "general" }))),
		).toBe("bot_general");
	});
});

describe("which bots have a workspace", () => {
	test("custom bots and the Assistant's account row; not Flux or a built-in kept on this computer", () => {
		expect(hasBotWorkspace({ system: null })).toBe(true);
		expect(hasBotWorkspace({ system: null, local: true })).toBe(true);
		expect(hasBotWorkspace({ system: "general" })).toBe(true);
		expect(hasBotWorkspace({ system: "general", local: true })).toBe(false);
		expect(hasBotWorkspace({ system: "flux" })).toBe(false);
	});
});
