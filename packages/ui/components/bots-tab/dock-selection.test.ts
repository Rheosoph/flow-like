import { describe, expect, test } from "bun:test";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	dockMoveRoute,
	dockPanelQuery,
	dockRoute,
	dockSelectionForBot,
	dockSelectionKey,
	dockSelectionQuery,
	parseDockPanel,
	resolveDockSelection,
	systemDockBot,
} from "./dock-selection";

function bot(id: string, overrides: Partial<IBot> = {}): IBot {
	return { id, system: null, ...overrides } as IBot;
}

const assistant = bot("bot_general", { system: "general" });
const flux = bot("bot_flux", { system: "flux" });
const scout = bot("bot_scout");
const localAssistant = bot("bot_general_here", {
	system: "general",
	local: true,
});
const localFlux = bot("bot_flux_here", { system: "flux", local: true });

describe("built-in bots on this computer beside the account's", () => {
	const signedIn = [localAssistant, assistant, flux, localFlux, scout];

	test("the account's row is the built-in's dock, wherever the list puts it", () => {
		expect(systemDockBot("general", signedIn)).toBe(assistant);
		expect(systemDockBot("flux", signedIn)).toBe(flux);
		expect(systemDockBot("general", [localAssistant])).toBe(localAssistant);
		expect(systemDockBot("general", [flux])).toBe(undefined);
	});

	test("the one on this computer opens its own chat, so the two never share a card or a chat", () => {
		expect(dockSelectionForBot(localAssistant, signedIn)).toEqual({
			type: "bot",
			botId: "bot_general_here",
		});
		expect(dockSelectionForBot(localFlux, signedIn)).toEqual({
			type: "bot",
			botId: "bot_flux_here",
		});
		expect(dockSelectionForBot(assistant, signedIn)).toEqual({
			type: "system",
			system: "general",
		});
		expect(
			dockSelectionKey(dockSelectionForBot(localAssistant, signedIn)),
		).not.toBe(dockSelectionKey(dockSelectionForBot(assistant, signedIn)));
		expect(
			resolveDockSelection(
				{ type: "bot", botId: "bot_general_here" },
				signedIn,
			),
		).toEqual({ type: "bot", botId: "bot_general_here" });
		expect(
			resolveDockSelection({ type: "bot", botId: "bot_general" }, signedIn),
		).toEqual({ type: "system", system: "general" });
	});

	test("signed out, the Assistant on this computer is the Assistant", () => {
		expect(
			dockSelectionForBot(localAssistant, [localAssistant, scout]),
		).toEqual({ type: "system", system: "general" });
		expect(dockSelectionForBot(assistant, undefined)).toEqual({
			type: "system",
			system: "general",
		});
	});
});

describe("the bot's panel in the URL", () => {
	test("?panel= names a known tab; anything else, or nothing, is a closed panel", () => {
		expect(parseDockPanel(new URLSearchParams("panel=details"))).toBe(
			"details",
		);
		expect(parseDockPanel(new URLSearchParams("panel=files"))).toBe("files");
		expect(parseDockPanel(new URLSearchParams("panel=memory"))).toBe("memory");
		expect(parseDockPanel(new URLSearchParams("panel=computer"))).toBe(
			"computer",
		);
		expect(parseDockPanel(new URLSearchParams("panel=bogus"))).toBeNull();
		expect(parseDockPanel(new URLSearchParams("pane=files"))).toBeNull();
		expect(parseDockPanel(new URLSearchParams())).toBeNull();
	});

	test("a closed panel leaves no ?panel=; other parameters stay", () => {
		expect(dockPanelQuery("data", "bot=bot_scout&table=facts")).toBe(
			"bot=bot_scout&table=facts&panel=data",
		);
		expect(dockPanelQuery(null, "bot=bot_scout&panel=data")).toBe(
			"bot=bot_scout",
		);
	});

	test("another bot or a conversation closes the panel and drops Data Studio's place", () => {
		const current =
			"bot=bot_scout&panel=data&table=facts&view=queries&scope=user&page=2&voice=1";
		expect(
			dockSelectionQuery({ type: "bot", botId: "bot_writer" }, current),
		).toBe("voice=1&bot=bot_writer");
		expect(
			dockSelectionQuery(
				{ type: "conversation", conversationId: "c1" },
				current,
			),
		).toBe("voice=1&conversation=c1");
	});

	test("the same bot keeps its panel and place", () => {
		expect(
			dockSelectionQuery(
				{ type: "bot", botId: "bot_scout" },
				"bot=bot_scout&panel=data&table=facts",
			),
		).toBe("panel=data&table=facts&bot=bot_scout");
	});

	test("a Flux bookmark keeps its session only while Flux stays selected", () => {
		const current = "bot=flux&session=chat-1&panel=memory&voice=1";
		expect(
			dockSelectionQuery({ type: "system", system: "flux" }, current),
		).toBe("session=chat-1&panel=memory&voice=1&bot=flux");
		expect(
			dockSelectionQuery({ type: "bot", botId: "bot_scout" }, current),
		).toBe("voice=1&bot=bot_scout");
		expect(
			dockSelectionQuery(
				{ type: "conversation", conversationId: "room-1" },
				current,
			),
		).toBe("voice=1&conversation=room-1");
	});

	test("links into the Bots tab carry the panel or the move review", () => {
		expect(dockRoute({ type: "system", system: "general" })).toBe("/bots");
		expect(dockRoute({ type: "bot", botId: "bot_scout" }, "files")).toBe(
			"/bots?bot=bot_scout&panel=files",
		);
		expect(dockRoute({ type: "system", system: "flux" }, "memory")).toBe(
			"/bots?bot=flux&panel=memory",
		);
		expect(dockMoveRoute({ type: "bot", botId: "bot_here" })).toBe(
			"/bots?bot=bot_here&move=local",
		);
	});
});

describe("a bot's direct conversation opens as the bot's chat", () => {
	const own = bot("bot_pip", { owner_id: "me" });
	const foreign = bot("bot_other", { owner_id: "anna" });
	const ownFlux = bot("bot_flux", { system: "flux", owner_id: "me" });
	const bots = [own, foreign, ownFlux, localAssistant];
	const direct = (id: string, partner: string) =>
		({
			id,
			kind: "direct",
			members: [
				{ principal_id: "me", kind: "user" },
				{ principal_id: partner, kind: "bot" },
			],
		}) as never;
	const conversations = [
		direct("c_pip", "bot_pip"),
		direct("c_other", "bot_other"),
		direct("c_flux", "bot_flux"),
		direct("c_local", "bot_general_here"),
		direct("c_anna", "anna"),
		{
			id: "c_room",
			kind: "group",
			members: [
				{ principal_id: "me", kind: "user" },
				{ principal_id: "bot_pip", kind: "bot" },
			],
		} as never,
	];
	const resolve = (conversationId: string) =>
		resolveDockSelection(
			{ type: "conversation", conversationId },
			bots,
			conversations,
			"me",
		);

	test("an owned bot's DM becomes its bot selection, built-ins by key", () => {
		expect(resolve("c_pip")).toEqual({ type: "bot", botId: "bot_pip" });
		expect(resolve("c_local")).toEqual({ type: "system", system: "general" });
	});

	test("Flux's DM stays a conversation: Flux's chat shows its own sessions, not the DM", () => {
		expect(resolve("c_flux")).toEqual({
			type: "conversation",
			conversationId: "c_flux",
		});
	});

	test("people, rooms, other people's bots and unknown ids stay conversations", () => {
		for (const id of ["c_anna", "c_room", "c_other", "missing"]) {
			expect(resolve(id)).toEqual({ type: "conversation", conversationId: id });
		}
	});
});
