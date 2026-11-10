// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import type { BotModelOption } from "./use-bot-models";

const mocks = vi.hoisted(() => ({
	catalog: vi.fn(),
	allows: vi.fn(),
	add: vi.fn(),
}));

vi.mock("./use-bot-models", () => ({
	useCatalogModelOptions: mocks.catalog,
}));
vi.mock("../../../../hooks/use-add-model-to-profile", () => ({
	useAddModelToProfile: () => ({ allows: mocks.allows, add: mocks.add }),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: unknown) =>
			typeof fallback === "string" ? fallback : key,
		i18n: { language: "en" },
	}),
}));

import { BotModelPicker } from "./model-picker";

function option(id: string, name: string, provider: string): BotModelOption {
	const bit = {
		id,
		type: IBitTypes.Stt,
		meta: { en: { name, description: `${name} transcribes speech.` } },
		parameters: { provider: { provider_name: provider } },
	} as unknown as IBit;
	return { id, label: name, bit, local: false, free: false };
}

const MINE = option("mine", "Profile Whisper", "hosted:openai");
const CATALOG = option("whisper", "Catalog Whisper", "hosted:groq");
const STT = { types: [IBitTypes.Stt] };

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	vi.clearAllMocks();
	mocks.catalog.mockReturnValue({
		options: [MINE, CATALOG],
		isLoading: false,
		isError: false,
	});
	mocks.allows.mockReturnValue(true);
	mocks.add.mockReturnValue(new Promise(() => {}));
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	document.body.innerHTML = "";
});

async function open(
	options: readonly BotModelOption[],
	onChange: (id: string | null) => void,
) {
	await act(async () => {
		root.render(
			<BotModelPicker
				label="Speech to text"
				value={null}
				options={options}
				catalog={STT}
				placeholder="Choose a model"
				onChange={onChange}
			/>,
		);
	});
	await act(async () => {
		(host.querySelector("button") as HTMLButtonElement).click();
	});
}

function choice(name: string): HTMLButtonElement {
	const found = [...document.body.querySelectorAll("button")].find((node) =>
		node.textContent?.includes(name),
	);
	if (!found) throw new Error(`Missing choice: ${name}`);
	return found;
}

test("with no fitting model in the profile the picker opens on the whole catalog", async () => {
	await open([], vi.fn());
	expect(document.body.textContent).toContain("None of your models fit here");
	expect(document.body.textContent).not.toContain("Your models");
	expect(mocks.catalog).toHaveBeenLastCalledWith(STT, true);
	expect(choice("Catalog Whisper")).toBeDefined();
});

test("a catalog model is used at once and added to the profile in the background", async () => {
	const onChange = vi.fn();
	await open([], onChange);
	await act(async () => choice("Catalog Whisper").click());
	expect(onChange).toHaveBeenCalledExactlyOnceWith("whisper");
	expect(mocks.add).toHaveBeenCalledExactlyOnceWith(CATALOG.bit);
});

test("a catalog model above the person's plan is neither used nor added", async () => {
	mocks.allows.mockReturnValue(false);
	const onChange = vi.fn();
	await open([], onChange);
	await act(async () => choice("Catalog Whisper").click());
	expect(onChange).not.toHaveBeenCalled();
	expect(mocks.add).not.toHaveBeenCalled();
});

test("the profile's models come first and the catalog loads only on its tab", async () => {
	const onChange = vi.fn();
	await open([MINE], onChange);
	expect(document.body.textContent).toContain("Your models");
	expect(mocks.catalog).toHaveBeenLastCalledWith(STT, false);
	expect(document.body.textContent).not.toContain("Catalog Whisper");
	await act(async () => choice("Profile Whisper").click());
	expect(onChange).toHaveBeenCalledExactlyOnceWith("mine");
	expect(mocks.add).not.toHaveBeenCalled();
});
