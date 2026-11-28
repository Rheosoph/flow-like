// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import type { BotLocation } from "../../../../state/backend-state/bot-state";
import type { BotModelOption } from "./use-bot-models";

const mocks = vi.hoisted(() => ({ options: vi.fn() }));

vi.mock("./use-bot-models", () => ({
	LLM_CATALOG: { types: [] },
	useBotModelOptions: mocks.options,
}));
vi.mock("./model-picker", () => ({ BotModelPicker: () => null }));
vi.mock("./decision-model-field", () => ({
	BotDecisionModelField: () => null,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string) => key,
		i18n: { language: "en" },
	}),
}));

import { BrainStep } from "./brain-step";

function option(id: string, provider: string, local = false): BotModelOption {
	const bit = {
		id,
		type: IBitTypes.Llm,
		meta: { en: { name: id, description: "" } },
		parameters: { provider: { provider_name: provider } },
	} as unknown as IBit;
	return { id, label: id, bit, local, free: false };
}

const OPTIONS = [
	option("hosted", "hosted:openrouter"),
	option("lan", "custom:ollama"),
	option("agent", "custom:claude-code"),
	option("gguf", "Local", true),
];

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	mocks.options.mockReturnValue({ options: OPTIONS, isLoading: false });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(() => {
	act(() => root.unmount());
	host.remove();
});

function render(location: BotLocation, primary: string) {
	act(() =>
		root.render(
			<BrainStep
				instructions=""
				model={{ primary, fallbacks: [] }}
				location={location}
				onInstructionsChange={() => {}}
				onModelChange={() => {}}
			/>,
		),
	);
	return host.textContent ?? "";
}

test("a device bot is told which models its device calls with the owner's key", () => {
	expect(render("device", "lan")).toContain("editor.brain.ownKeyOnDevice");
	for (const primary of ["agent", "gguf"]) {
		const text = render("device", primary);
		expect(text, primary).toContain("editor.brain.notOnDevice");
		expect(text, primary).not.toContain("editor.brain.ownKeyOnDevice");
	}
	const hosted = render("device", "hosted");
	expect(hosted).not.toContain("editor.brain.notOnDevice");
	expect(hosted).not.toContain("editor.brain.ownKeyOnDevice");
});

test("only device bots get the device notices", () => {
	for (const location of ["server", "desktop"] as const) {
		const text = render(location, "lan");
		expect(text, location).not.toContain("editor.brain.ownKeyOnDevice");
		expect(text, location).not.toContain("editor.brain.notOnDevice");
	}
	expect(render("server", "gguf")).toContain("editor.brain.localOnServer");
});
