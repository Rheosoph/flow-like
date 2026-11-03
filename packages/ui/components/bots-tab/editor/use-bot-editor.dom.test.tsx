import { afterEach, describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { Window } from "happy-dom";
import { act } from "react";
import type { IBot } from "../../../state/backend-state/bot-state";
import type { BotEditorMode } from "./bot-editor-steps";
import { findBotPreset } from "./bot-presets";
import type { BotEditor } from "./use-bot-editor";

type BotEditorAction = {
	[Name in keyof BotEditor as BotEditor[Name] extends (
		...args: never[]
	) => unknown
		? Name
		: never]: BotEditor[Name];
};

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

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
		capabilities: [],
		profile_id: null,
		location: "desktop",
		device_id: null,
		created_at: "2026-11-01T00:00:00Z",
		updated_at: "2026-11-01T00:00:00Z",
		...overrides,
	};
}

function preset(id: string) {
	const found = findBotPreset(id);
	if (!found) throw new Error(`Preset ${id} is missing`);
	return found;
}

/** Puts a happy-dom window on `globalThis` before react-dom loads; returns the undo. */
function installDom(window: Window) {
	const globals: Record<string, unknown> = {
		window,
		document: window.document,
		navigator: window.navigator,
		Node: window.Node,
		Element: window.Element,
		HTMLElement: window.HTMLElement,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = new Map<string, PropertyDescriptor | undefined>();
	for (const key of Object.keys(globals))
		previous.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
	Object.assign(globalThis, globals);
	return function restoreDom() {
		for (const [key, descriptor] of previous) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
}

async function renderEditor(mode: BotEditorMode, original: IBot | null) {
	const window = new Window();
	const restoreDom = installDom(window);
	createI18n({ language: "en" });
	const { createRoot } = await import("react-dom/client");
	const { useBotEditor } = await import("./use-bot-editor");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	const state: { editor?: BotEditor } = {};
	function Probe() {
		state.editor = useBotEditor(mode, original);
		return null;
	}
	await act(async () => root.render(<Probe />));
	cleanup = async function unmount() {
		await act(async () => root.unmount());
		await window.happyDOM.close();
		restoreDom();
	};
	function current() {
		if (!state.editor) throw new Error("The editor did not render");
		return state.editor;
	}
	/** Calls one of the editor's actions inside `act`, so the next read sees its result. */
	function perform<Name extends keyof BotEditorAction>(
		name: Name,
		...args: Parameters<BotEditorAction[Name]>
	) {
		const action = current()[name] as (
			...args: Parameters<BotEditorAction[Name]>
		) => void;
		return act(async () => {
			action(...args);
		});
	}
	return { current, perform };
}

describe("useBotEditor in create mode", () => {
	test("leaving Describe sets the bot up from the best match", async () => {
		const { current, perform } = await renderEditor("create", null);
		await perform("setDescribeText", "book meetings in my calendar");
		expect(current().candidate.presetId).toBe("secretary");
		expect(current().draft.presetId).toBeNull();

		await perform("goTo", "look");
		const { draft } = current();
		expect(current().step).toBe("look");
		expect(draft.presetId).toBe("secretary");
		expect(draft.name).toBe("Secretary");
		expect(draft.description).toBe("book meetings in my calendar");
		expect(draft.instructions.length).toBeGreaterThan(0);
		expect(draft.capabilities).toEqual([
			...preset("secretary").capabilities,
			"memory",
		]);
		expect(current().dirty).toBe(true);
	});

	test("a picked preset brings its own description when nothing was typed", async () => {
		const { current, perform } = await renderEditor("create", null);
		await perform("pickPreset", preset("coder"));
		expect(current().draft.description).toBe(
			"Writes and runs code and uses the command line on your computer.",
		);
		expect(current().draft.location).toBe("desktop");

		await perform("goTo", "location");
		await perform("setLocation", "server");
		expect(current().draft.capabilities).toEqual(["code_execution", "memory"]);
		expect(current().removedCapabilities).toEqual(["command_line"]);
	});

	test("unrelated words start a blank bot that keeps them as its description", async () => {
		const { current, perform } = await renderEditor("create", null);
		await perform("setDescribeText", "purple elephants");
		expect(current().matches).toEqual([]);
		await perform("goTo", "look");
		expect(current().draft.presetId).toBeNull();
		expect(current().draft.description).toBe("purple elephants");
		expect(current().draft.capabilities).toEqual(["memory"]);
	});

	test("starting from scratch opens the look", async () => {
		const { current, perform } = await renderEditor("create", null);
		await perform("setDescribeText", "a calendar helper");
		await perform("startFromScratch");
		expect(current().step).toBe("look");
		expect(current().draft.presetId).toBeNull();
		expect(current().draft.description).toBe("a calendar helper");
	});

	test("newer words become the description until the user edits it", async () => {
		const { current, perform } = await renderEditor("create", null);
		await perform("pickPreset", preset("research_digest"));
		await perform("setDescribeText", "watch competitor news");
		await perform("goTo", "look");
		expect(current().draft.presetId).toBe("research_digest");
		expect(current().draft.description).toBe("watch competitor news");

		await perform("setIdentity", { description: "Mine" });
		await perform("goTo", "describe");
		await perform("setDescribeText", "something else");
		await perform("goTo", "abilities");
		expect(current().draft.description).toBe("Mine");
	});
});

describe("useBotEditor in edit mode", () => {
	test("is clean until something changes, then sends only that", async () => {
		const { current, perform } = await renderEditor("edit", bot());
		expect(current().steps[0]).toBe("look");
		expect(current().steps).toContain("secrets");
		expect(current().dirty).toBe(false);
		await perform("setIdentity", { name: "Scout II" });
		expect(current().dirty).toBe(true);
		expect(current().updateRequest).toEqual({ name: "Scout II" });
	});

	test("moving to the cloud and back brings the local abilities back", async () => {
		const { current, perform } = await renderEditor(
			"edit",
			bot({ capabilities: ["internet", "computer_use", "command_line"] }),
		);
		await perform("setLocation", "server");
		expect(current().draft.capabilities).toEqual(["internet"]);
		expect(current().removedCapabilities).toEqual([
			"computer_use",
			"command_line",
		]);
		expect(current().restoredCapabilities).toEqual([]);

		await perform("setLocation", "desktop");
		expect(current().draft.capabilities).toEqual([
			"internet",
			"computer_use",
			"command_line",
		]);
		expect(current().removedCapabilities).toEqual([]);
		expect(current().restoredCapabilities).toEqual([
			"computer_use",
			"command_line",
		]);
		expect(current().updateRequest).toEqual({});
		expect(current().dirty).toBe(false);
	});

	test("an ability switched off by hand stays off after another round trip", async () => {
		const { current, perform } = await renderEditor(
			"edit",
			bot({ capabilities: ["computer_use", "command_line"] }),
		);
		await perform("setLocation", "server");
		await perform("setLocation", "device");
		await perform("toggleCapability", "command_line", false);
		expect(current().restoredCapabilities).toEqual(["computer_use"]);

		await perform("setLocation", "server");
		expect(current().removedCapabilities).toEqual(["computer_use"]);
		await perform("setLocation", "desktop");
		expect(current().draft.capabilities).toEqual(["computer_use"]);
		expect(current().restoredCapabilities).toEqual(["computer_use"]);
		expect(current().updateRequest).toEqual({ capabilities: ["computer_use"] });
	});

	test("Flux keeps its host and abilities but edits the rest", async () => {
		const { current, perform } = await renderEditor(
			"edit",
			bot({ system: "flux", name: null }),
		);
		expect(current().flowPilot).toBe(true);
		expect(current().fixedLocation).toBe(true);
		expect(current().steps).not.toContain("location");

		await perform("setLocation", "server");
		await perform("toggleCapability", "internet", true);
		expect(current().draft.location).toBe("desktop");
		expect(current().draft.capabilities).toEqual([]);
		expect(current().dirty).toBe(false);

		await perform("setIdentity", { name: "Builder" });
		await perform("setInstructions", "Explain each change.");
		await perform("setModel", { primary: "model_a", fallbacks: [] });
		expect(current().updateRequest).toEqual({
			name: "Builder",
			instructions: "Explain each change.",
			model: { primary: "model_a", fallbacks: [] },
		});
	});

	test("a Flux duplicate keeps its abilities wherever it moves", async () => {
		const { current, perform } = await renderEditor(
			"edit",
			bot({ capabilities: ["flowpilot"] }),
		);
		expect(current().flowPilot).toBe(true);
		expect(current().fixedLocation).toBe(false);
		expect(current().steps).toContain("location");

		await perform("toggleCapability", "computer_use", true);
		await perform("toggleCapability", "flowpilot", false);
		await perform("setLocation", "server");
		expect(current().draft.capabilities).toEqual(["flowpilot"]);
		expect(current().removedCapabilities).toEqual([]);
		expect(current().updateRequest).toEqual({ location: "server" });
	});

	test("the memory model is sent only when it changes", async () => {
		const { current, perform } = await renderEditor(
			"edit",
			bot({ embedding_model: "embed_a" }),
		);
		expect(current().draft.embeddingModel).toBe("embed_a");

		await perform("setEmbeddingModel", null);
		expect(current().updateRequest).toEqual({ embedding_model: null });
		expect(current().dirty).toBe(true);

		await perform("setEmbeddingModel", "embed_b");
		expect(current().updateRequest).toEqual({ embedding_model: "embed_b" });

		await perform("setEmbeddingModel", "embed_a");
		expect(current().updateRequest).toEqual({});
		expect(current().dirty).toBe(false);
	});
});
