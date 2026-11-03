import { describe, expect, test } from "bun:test";
import type { IBot } from "../../../state/backend-state/bot-state";
import { defaultBotCatalog } from "../../bots/catalog";
import {
	botDraftFromBot,
	botDraftFromPreset,
	buildCreateBotRequest,
	buildUpdateBotRequest,
	createEmptyBotDraft,
	getBotDraftProblems,
	hasFlowPilot,
	normalizeBotModelPolicy,
	setBotDraftCapability,
	setBotDraftLocation,
} from "./bot-draft";
import { BOT_OWNED_PACK_IDS } from "./bot-look";
import { findBotPreset } from "./bot-presets";

const texts = { name: "Coder", description: "Codes", instructions: "Be kind" };

function bot(overrides: Partial<IBot> = {}): IBot {
	return {
		id: "bot_1",
		owner_id: "user_1",
		system: null,
		preset_id: null,
		name: "Scout",
		description: null,
		instructions: null,
		appearance: { kind: "mochi" },
		model: { primary: null, fallbacks: [] },
		classification: { pii: false },
		capabilities: ["internet"],
		profile_id: null,
		location: "server",
		device_id: null,
		created_at: "2026-11-01T00:00:00Z",
		updated_at: "2026-11-01T00:00:00Z",
		...overrides,
	};
}

const lockedKind = Object.values(defaultBotCatalog.packs).find(
	(pack) => !BOT_OWNED_PACK_IDS.includes(pack.id),
)?.shapeIds[0] as string;

describe("drafts", () => {
	test("a blank draft only needs a name and starts with Memory on", () => {
		const draft = createEmptyBotDraft();
		expect(draft.appearance.kind).toBe(defaultBotCatalog.defaultKind);
		expect(draft.capabilities).toEqual(["memory"]);
		expect(
			getBotDraftProblems(draft, { system: null, checkLook: true }),
		).toEqual([{ code: "name_required" }]);
		expect(
			buildCreateBotRequest({ ...draft, name: "Scout" }).capabilities,
		).toEqual(["memory"]);
	});

	test("a preset fills the draft, starts with Memory on and keeps only what its host can do", () => {
		const preset = findBotPreset("coder");
		if (!preset) throw new Error("coder preset missing");
		const draft = botDraftFromPreset({ ...preset, location: "server" }, texts);
		expect(draft.presetId).toBe("coder");
		expect(draft.name).toBe("Coder");
		expect(draft.instructions).toBe("Be kind");
		expect(draft.capabilities).toEqual(["code_execution", "memory"]);
		expect(draft.appearance).not.toBe(preset.appearance);
	});

	test("Memory switched off stays off", () => {
		const draft = setBotDraftCapability(createEmptyBotDraft(), "memory", false);
		expect(draft.capabilities).toEqual([]);
		expect(
			buildCreateBotRequest({ ...draft, name: "Scout" }).capabilities,
		).toEqual([]);
	});

	test("moving to the server switches off local-only capabilities", () => {
		const draft = {
			...createEmptyBotDraft(),
			location: "desktop" as const,
			capabilities: ["internet", "browser_use", "command_line"] as const,
		};
		const moved = setBotDraftLocation(
			{ ...draft, capabilities: [...draft.capabilities] },
			"server",
		);
		expect(moved.removed).toEqual(["browser_use", "command_line"]);
		expect(moved.draft.capabilities).toEqual(["internet"]);
		expect(moved.draft.location).toBe("server");
	});

	test("moving back from the server switches the remembered capabilities on again", () => {
		const draft = {
			...createEmptyBotDraft(),
			location: "desktop" as const,
			capabilities: ["internet", "computer_use"] as IBot["capabilities"],
		};
		const away = setBotDraftLocation(draft, "server");
		expect(away.remembered).toEqual(["computer_use"]);
		expect(away.restored).toEqual([]);

		const back = setBotDraftLocation(away.draft, "device", away.remembered);
		expect(back.draft.capabilities).toEqual(["internet", "computer_use"]);
		expect(back.draft.location).toBe("device");
		expect(back.removed).toEqual([]);
		expect(back.restored).toEqual(["computer_use"]);
		expect(back.remembered).toEqual([]);
	});

	test("capabilities keep the wire order and respect the location", () => {
		let draft = createEmptyBotDraft();
		draft = setBotDraftCapability(draft, "webhook", true);
		draft = setBotDraftCapability(draft, "internet", true);
		draft = setBotDraftCapability(draft, "computer_use", true);
		expect(draft.capabilities).toEqual(["internet", "memory", "webhook"]);
		draft = setBotDraftCapability(draft, "internet", false);
		expect(draft.capabilities).toEqual(["memory", "webhook"]);
	});

	test("flowpilot is never switched, and a bot that has it keeps every ability", () => {
		const blank = createEmptyBotDraft();
		expect(setBotDraftCapability(blank, "flowpilot", true)).toBe(blank);

		const pilot = { ...blank, capabilities: ["flowpilot"] as const };
		const draft = { ...pilot, capabilities: [...pilot.capabilities] };
		expect(setBotDraftCapability(draft, "flowpilot", false)).toBe(draft);
		expect(setBotDraftCapability(draft, "internet", true)).toBe(draft);
	});

	test("Flux and its duplicates have FlowPilot, older Flux rows included", () => {
		expect(hasFlowPilot(bot({ system: "flux", capabilities: [] }))).toBe(true);
		expect(hasFlowPilot(bot({ capabilities: ["flowpilot"] }))).toBe(true);
		expect(hasFlowPilot(bot({ system: "general" }))).toBe(false);
	});

	test("model policies drop duplicates, the primary and extra fallbacks", () => {
		expect(
			normalizeBotModelPolicy({
				primary: " a ",
				fallbacks: ["a", "b", "b", "c", "d", "e"],
			}),
		).toEqual({ primary: "a", fallbacks: ["b", "c", "d"] });
	});
});

describe("requests", () => {
	test("create sends cleaned values and drops unused ids", () => {
		const request = buildCreateBotRequest({
			...createEmptyBotDraft(),
			name: "  Scout ",
			description: "   ",
			capabilities: ["selected_apps", "internet"],
			profileId: "profile_1",
			deviceId: "device_1",
		});
		expect(request.name).toBe("Scout");
		expect(request.description).toBeNull();
		expect(request.capabilities).toEqual(["internet", "selected_apps"]);
		expect(request.profile_id).toBeNull();
		expect(request.device_id).toBeNull();
		expect(request).not.toHaveProperty("embedding_model");
	});

	test("create sends the memory model only when one is picked", () => {
		const request = buildCreateBotRequest({
			...createEmptyBotDraft(),
			name: "Scout",
			embeddingModel: "embed_a",
		});
		expect(request.embedding_model).toBe("embed_a");
	});

	test("update sends the memory model when it changes, null to let the host choose", () => {
		const original = bot({ embedding_model: "embed_a" });
		const draft = botDraftFromBot(original);
		expect(buildUpdateBotRequest(original, draft)).toEqual({});
		expect(
			buildUpdateBotRequest(original, { ...draft, embeddingModel: null }),
		).toEqual({ embedding_model: null });
		const unset = bot();
		expect(
			buildUpdateBotRequest(unset, {
				...botDraftFromBot(unset),
				embeddingModel: "embed_b",
			}),
		).toEqual({ embedding_model: "embed_b" });
	});

	test("update sends nothing when nothing changed", () => {
		const original = bot();
		expect(buildUpdateBotRequest(original, botDraftFromBot(original))).toEqual(
			{},
		);
	});

	test("update sends only the changed fields", () => {
		const original = bot({ capabilities: ["internet", "email"] });
		const draft = {
			...botDraftFromBot(original),
			description: "Finds things",
			capabilities: ["email", "internet"] as IBot["capabilities"],
			location: "device" as const,
			deviceId: "device_1",
		};
		expect(buildUpdateBotRequest(original, draft)).toEqual({
			description: "Finds things",
			location: "device",
			device_id: "device_1",
		});
	});

	test("clearing a built-in bot's name sends null", () => {
		const original = bot({ system: "general", name: "Helper" });
		const draft = { ...botDraftFromBot(original), name: "  " };
		expect(buildUpdateBotRequest(original, draft)).toEqual({ name: null });
		expect(
			getBotDraftProblems(draft, { system: "general", checkLook: false }),
		).toEqual([]);
	});

	test("Flux never sends capabilities", () => {
		const original = bot({ system: "flux", capabilities: ["internet"] });
		const draft = setBotDraftCapability(
			botDraftFromBot(original),
			"email",
			true,
		);
		expect(buildUpdateBotRequest(original, draft)).toEqual({});
	});

	test("Flux never sends its host; a duplicate sends its host but not its abilities", () => {
		const flux = bot({ system: "flux", capabilities: [] });
		expect(
			buildUpdateBotRequest(flux, {
				...botDraftFromBot(flux),
				location: "device",
				deviceId: "device_1",
				name: "Builder",
			}),
		).toEqual({ name: "Builder" });

		const copy = bot({ capabilities: ["flowpilot"] });
		expect(
			buildUpdateBotRequest(copy, {
				...botDraftFromBot(copy),
				location: "desktop",
				capabilities: ["internet"],
			}),
		).toEqual({ location: "desktop" });
	});

	test("switching profile apps off clears the profile", () => {
		const original = bot({
			capabilities: ["profile_apps"],
			profile_id: "profile_1",
		});
		const draft = setBotDraftCapability(
			botDraftFromBot(original),
			"profile_apps",
			false,
		);
		expect(buildUpdateBotRequest(original, draft)).toEqual({
			capabilities: [],
			profile_id: null,
		});
	});
});

describe("problems", () => {
	test("a look from a pack nobody owns blocks saving until it changes", () => {
		const draft = {
			...createEmptyBotDraft(),
			name: "Scout",
			appearance: { kind: lockedKind },
		};
		const problems = getBotDraftProblems(draft, {
			system: null,
			checkLook: true,
		});
		expect(problems.map((problem) => problem.code)).toEqual(["look_locked"]);
		expect(
			getBotDraftProblems(draft, { system: null, checkLook: false }),
		).toEqual([]);
	});

	test("missing profile, device and too long texts are reported", () => {
		const draft = {
			...createEmptyBotDraft(),
			name: "x".repeat(65),
			instructions: "y".repeat(8001),
			capabilities: ["profile_apps"] as IBot["capabilities"],
			location: "device" as const,
		};
		expect(
			getBotDraftProblems(draft, { system: null, checkLook: true }).map(
				(problem) => problem.code,
			),
		).toEqual([
			"name_too_long",
			"instructions_too_long",
			"profile_required",
			"device_required",
		]);
	});
});
