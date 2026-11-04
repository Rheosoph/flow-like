import { describe, expect, test } from "bun:test";
import type { IBot, IBotMoveIssue } from "../../state/backend-state/bot-state";
import { defaultBotCatalog } from "../bots/catalog";
import {
	BOT_LOOK_NOT_OWNED,
	BOT_MODEL_UNAVAILABLE,
	BOT_SECRET_UNREADABLE,
	clientMoveIssues,
	mergeMoveIssues,
	moveFixRequest,
	moveFixes,
	staysOnThisComputer,
} from "./move-checks";

function bot(overrides: Partial<IBot> = {}): IBot {
	return {
		id: "bot_local",
		owner_id: "local",
		system: null,
		preset_id: null,
		name: "Scout",
		description: null,
		instructions: null,
		appearance: { kind: "mochi" },
		model: { primary: "model-a", fallbacks: ["model-b", "model-c"] },
		classification: { pii: false },
		capabilities: ["internet", "profile_apps"],
		profile_id: "profile-old",
		location: "desktop",
		device_id: null,
		created_at: "",
		updated_at: "",
		local: true,
		...overrides,
	};
}

function issue(
	code: string,
	ref_id: string | null = null,
	part: IBotMoveIssue["part"] = "bot",
): IBotMoveIssue {
	return { part, code, message: code, ref_id };
}

describe("clientMoveIssues", () => {
	test("names every model the account doesn't offer, primary first", () => {
		const issues = clientMoveIssues(bot(), new Set(["model-b"]));
		expect(
			issues.map(({ part, code, ref_id }) => [part, code, ref_id]),
		).toEqual([
			["model", BOT_MODEL_UNAVAILABLE, "model-a"],
			["model", BOT_MODEL_UNAVAILABLE, "model-c"],
		]);
	});

	test("an automatic model and valid looks pass", () => {
		const automatic = bot({ model: { primary: null, fallbacks: [] } });
		expect(clientMoveIssues(automatic, new Set())).toEqual([]);
	});

	test("every catalog character can move without purchases", () => {
		for (const kind of Object.keys(defaultBotCatalog.shapes)) {
			expect(
				clientMoveIssues(
					bot({
						appearance: { kind },
						model: { primary: null, fallbacks: [] },
					}),
					new Set(),
				),
			).toEqual([]);
		}
	});

	test("packed equipment and premium colors can move without purchases", () => {
		expect(
			clientMoveIssues(
				bot({
					appearance: {
						kind: "lava",
						equipment: { head: "crown" },
						colors: {
							body: { kind: "swatch", colorId: "holo-aurora" },
							accent: { kind: "hex", value: "#123456", packId: "color-lab" },
						},
					},
					model: { primary: null, fallbacks: [] },
				}),
				new Set(),
			),
		).toEqual([]);
	});

	test("an unavailable look still blocks the move", () => {
		const issues = clientMoveIssues(
			bot({
				appearance: { kind: "missing-character" },
				model: { primary: null, fallbacks: [] },
			}),
			new Set(),
		);
		expect(issues).toHaveLength(1);
		expect(issues[0]).toMatchObject({
			part: "look",
			code: BOT_LOOK_NOT_OWNED,
			ref_id: null,
		});
		expect(issues[0]?.message.length).toBeGreaterThan(0);
	});
});

describe("mergeMoveIssues", () => {
	test("keeps server and client issues once each, server first", () => {
		const app = issue("BOT_APP_UNAVAILABLE", "app_1", "app");
		const model = issue(BOT_MODEL_UNAVAILABLE, "model-a", "model");
		expect(mergeMoveIssues([app, model], [{ ...model, message: "x" }])).toEqual(
			[app, model],
		);
	});
});

describe("moveFixes", () => {
	const kinds = (found: IBotMoveIssue) =>
		moveFixes(found, bot()).map((fix) => fix.kind);

	test("offers the fix each issue needs", () => {
		expect(moveFixes(issue("BOT_APP_UNAVAILABLE", "app_1"), bot())).toEqual([
			{ kind: "detach_app", appId: "app_1" },
		]);
		expect(kinds(issue("BOT_PROFILE_UNAVAILABLE"))).toEqual([
			"use_profile",
			"drop_profile_apps",
		]);
		expect(kinds(issue(BOT_MODEL_UNAVAILABLE, "model-a"))).toEqual([
			"automatic_model",
			"open_settings",
		]);
		expect(moveFixes(issue(BOT_MODEL_UNAVAILABLE, "model-c"), bot())).toEqual([
			{ kind: "drop_fallback", modelId: "model-c" },
			{ kind: "open_settings" },
		]);
		expect(kinds(issue(BOT_LOOK_NOT_OWNED))).toEqual(["open_settings"]);
		expect(kinds(issue("BOT_CAPABILITY_LOCATION"))).toEqual(["open_settings"]);
		expect(
			moveFixes(issue(BOT_SECRET_UNREADABLE, "OLD_TOKEN", "secret"), bot()),
		).toEqual([{ kind: "remove_secret", name: "OLD_TOKEN" }]);
		expect(
			moveFixRequest({ kind: "remove_secret", name: "X" }, bot(), null),
		).toBeNull();
	});

	test("a full account leaves the bot here, with nothing to fix", () => {
		expect(staysOnThisComputer(issue("BOT_LIMIT_REACHED"))).toBe(true);
		expect(kinds(issue("BOT_LIMIT_REACHED"))).toEqual([]);
		expect(staysOnThisComputer(issue("BOT_APP_UNAVAILABLE"))).toBe(false);
	});

	test("so does a plan whose bots on the server are used up", () => {
		expect(staysOnThisComputer(issue("BOT_SERVER_LIMIT_REACHED"))).toBe(true);
		expect(kinds(issue("BOT_SERVER_LIMIT_REACHED"))).toEqual([]);
	});
});

describe("moveFixRequest", () => {
	test("turns fixes into bot updates", () => {
		expect(
			moveFixRequest({ kind: "use_profile" }, bot(), "profile-now"),
		).toEqual({ profile_id: "profile-now" });
		expect(moveFixRequest({ kind: "use_profile" }, bot(), null)).toBeNull();
		expect(moveFixRequest({ kind: "drop_profile_apps" }, bot(), null)).toEqual({
			capabilities: ["internet"],
			profile_id: null,
		});
		expect(moveFixRequest({ kind: "automatic_model" }, bot(), null)).toEqual({
			model: { primary: null, fallbacks: ["model-b", "model-c"] },
		});
		expect(
			moveFixRequest(
				{ kind: "drop_fallback", modelId: "model-b" },
				bot(),
				null,
			),
		).toEqual({ model: { primary: "model-a", fallbacks: ["model-c"] } });
		expect(
			moveFixRequest({ kind: "detach_app", appId: "app_1" }, bot(), null),
		).toBeNull();
	});
});
