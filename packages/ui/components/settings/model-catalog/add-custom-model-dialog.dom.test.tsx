// @vitest-environment happy-dom
import { type ReactNode, act, useState } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type {
	ExternalModelCatalog,
	ExternalModelOption,
	ExternalModelProvider,
} from "../../../lib/bit/external-model-providers";
import { type IBit, IBitTypes } from "../../../lib/schema/bit/bit";
import { AddCustomModelDialog } from "./add-custom-model-dialog";

const harness = vi.hoisted(() => ({
	backend: {} as Record<string, unknown>,
	close: vi.fn(),
	toastError: vi.fn(),
}));

function translate(
	key: string,
	fallback?: string,
	values?: Record<string, unknown>,
) {
	return (fallback ?? key).replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
		String(values?.[name] ?? ""),
	);
}

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: translate }),
	i18n: { t: translate },
	Trans: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("../../../state/backend-state", () => ({
	useBackend: () => harness.backend,
}));
vi.mock("../../../hooks", () => ({
	useInvalidateInvoke: () => async () => {},
}));
vi.mock("../../../state/download-manager", () => ({
	useDownloadManager: () => async () => {},
}));
vi.mock("sonner", () => ({
	toast: { success: () => {}, error: harness.toastError },
}));
vi.mock("../../bits/bit-editor-dialog", () => ({
	BitEditorDialog: ({ bit }: { bit: IBit }) => (
		<div data-testid="generic-bit-editor">{bit.id}</div>
	),
}));
vi.mock("./add-forecasting-model-dialog", () => ({
	AddForecastingModelDialog: () => null,
}));
// Keep the actual dialog and form controls without loading unrelated editors from the UI barrel.
vi.mock("../../ui", async () =>
	Object.assign(
		{},
		...(await Promise.all([
			import("../../ui/avatar"),
			import("../../ui/badge"),
			import("../../ui/button"),
			import("../../ui/collapsible"),
			import("../../ui/data-classification"),
			import("../../ui/dialog"),
			import("../../ui/input"),
			import("../../ui/label"),
			import("../../ui/slider"),
			import("../../ui/switch"),
			import("../../ui/textarea"),
		])),
		{ formatContextLength: (length: number) => String(length) },
	),
);

const PROFILE = { hub_profile: { id: "current-profile", bits: [] } };
const FIRST = {
	id: "runtime-model-a",
	name: "Runtime model A",
	description: "Description reported by the installed runtime.",
	contextWindow: 196_608,
	supportsVision: true,
	supportedReasoningEfforts: [
		{ id: "low", name: "Low" },
		{ id: "high", name: "High" },
	],
	defaultReasoningEffort: "high",
	isDefault: true,
};
const SECOND = {
	id: "runtime-model-b",
	name: "Runtime model B",
	description: "Another description from the runtime.",
	contextWindow: 262_144,
	supportsVision: false,
};

function catalog(
	models: ExternalModelOption[] = [FIRST, SECOND],
): ExternalModelCatalog {
	return { available: true, authenticated: true, models };
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

const discover =
	vi.fn<(provider: ExternalModelProvider) => Promise<ExternalModelCatalog>>();
const save =
	vi.fn<(bit: IBit, secrets?: Record<string, unknown>) => Promise<IBit>>();
const addToProfile = vi.fn();
let root: Root;
let host: HTMLDivElement;

function DialogHost({
	webMode = false,
	existingBit,
}: { webMode?: boolean; existingBit?: IBit }) {
	const [open, setOpen] = useState(true);
	return (
		<>
			<button type="button" onClick={() => setOpen(true)}>
				Reopen setup
			</button>
			<AddCustomModelDialog
				open={open}
				webMode={webMode}
				existingBit={existingBit}
				onOpenChange={(value) => {
					harness.close(value);
					setOpen(value);
				}}
			/>
		</>
	);
}

function dialog() {
	const element = document.querySelector<HTMLElement>("[role='dialog']");
	if (!element) throw new Error("Model setup dialog is missing");
	return element;
}

function field(id: string) {
	const element = dialog().querySelector<
		HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
	>(`#${id}`);
	if (!element) throw new Error(`Field ${id} is missing`);
	return element;
}

function button(label: string, within: ParentNode = dialog()) {
	const element = [
		...within.querySelectorAll<HTMLButtonElement>("button"),
	].find(
		(candidate) =>
			candidate.textContent?.replace(/\s+/g, " ").trim() === label ||
			candidate.getAttribute("aria-label") === label,
	);
	if (!element) throw new Error(`Button ${label} is missing`);
	return element;
}

async function click(element: HTMLButtonElement) {
	expect(element.disabled).toBe(false);
	await act(async () => element.click());
}

async function enter(id: string, value: string) {
	const element = field(id);
	const prototype =
		element instanceof HTMLSelectElement
			? HTMLSelectElement.prototype
			: element instanceof HTMLTextAreaElement
				? HTMLTextAreaElement.prototype
				: HTMLInputElement.prototype;
	await act(async () => {
		Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(
			element,
			value,
		);
		element.dispatchEvent(new Event("input", { bubbles: true }));
		element.dispatchEvent(new Event("change", { bubbles: true }));
	});
}

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function chooseProvider(provider: ExternalModelProvider) {
	const label = provider === "claude-code" ? "Claude Code" : "Codex (ChatGPT)";
	const tile = [...dialog().querySelectorAll<HTMLButtonElement>("button")].find(
		(candidate) => candidate.querySelector("p")?.textContent === label,
	);
	if (!tile) throw new Error(`Provider ${label} is missing`);
	await click(tile);
	await settle();
}

function modelOptions() {
	return [...field("discovered-model").querySelectorAll("option")]
		.map((option) => option.value)
		.filter(Boolean);
}

function savedBit() {
	const bit = save.mock.calls.at(-1)?.[0];
	if (!bit) throw new Error("No custom Bit was saved");
	return bit;
}

async function mount(webMode = false, existingBit?: IBit) {
	await act(async () =>
		root.render(<DialogHost webMode={webMode} existingBit={existingBit} />),
	);
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.clearAllMocks();
	discover.mockReset().mockResolvedValue(catalog());
	save.mockReset().mockImplementation(async (bit) => bit);
	addToProfile.mockResolvedValue(undefined);
	harness.backend = {
		capabilities: () => ({
			canHostLlamaCPP: false,
			canHostMLX: false,
			canUseNativeAgentProviders: true,
		}),
		bitState: {
			listExternalModels: discover,
			upsertCustomBit: save,
			addBit: addToProfile,
			listCustomBits: async () => [],
			getProfileBits: async () => [],
			searchBits: async () => [],
		},
		userState: {
			getSettingsProfile: async () => PROFILE,
			getProfile: async () => PROFILE.hub_profile,
		},
	};
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await settle();
	host.remove();
});

test.each(["claude-code", "codex"] as const)(
	"%s discovers on entry and saves the chosen model with its reported metadata",
	async (provider) => {
		const pending = deferred<ExternalModelCatalog>();
		discover.mockReturnValueOnce(pending.promise);
		await mount();
		await chooseProvider(provider);
		expect(discover).toHaveBeenCalledExactlyOnceWith(provider);
		expect(save).not.toHaveBeenCalled();
		expect(button("Add model").disabled).toBe(true);
		expect(dialog().querySelector("#custom-model-model_id")).toBeNull();
		await act(async () =>
			pending.resolve(
				catalog([{ id: "default", name: "CLI default" }, FIRST, SECOND]),
			),
		);
		expect(modelOptions()).toEqual([FIRST.id, SECOND.id]);
		expect(field("discovered-model").value).toBe("");
		await enter("discovered-model", FIRST.id);
		const expectedTags = [
			provider,
			"chat",
			"reasoning",
			...(provider === "codex" ? ["vision"] : []),
		];
		const expectedIcon =
			provider === "claude-code"
				? "/provider/anthropic.webp"
				: "/provider/gpt.webp";
		expect(field("custom-model-name").value).toBe(FIRST.name);
		expect(field("custom-model-description").value).toBe(FIRST.description);
		expect(field("custom-model-icon").value).toBe(expectedIcon);
		expect(field("custom-model-context").value).toBe(
			String(FIRST.contextWindow),
		);
		expect(
			field("custom-model-tags")
				.value.split(",")
				.map((tag) => tag.trim()),
		).toEqual(expectedTags);
		expect(
			dialog()
				.querySelector("#custom-model-vision")
				?.getAttribute("aria-checked"),
		).toBe(String(provider === "codex"));
		await click(button("Add model"));
		expect(save).toHaveBeenCalledTimes(1);
		const bit = savedBit();
		expect(bit.type).toBe(provider === "codex" ? "Vlm" : "Llm");
		expect(bit.meta.en).toMatchObject({
			name: FIRST.name,
			description: FIRST.description,
			icon: expectedIcon,
			tags: expectedTags,
		});
		expect(bit.parameters).toMatchObject({
			context_length: FIRST.contextWindow,
			provider: { provider_name: `custom:${provider}`, model_id: FIRST.id },
		});
		expect(
			Object.values(bit.parameters?.model_classification ?? {}).every(
				(score) => score === 0.5,
			),
		).toBe(true);
		expect(save.mock.calls[0]?.[1]).toBeUndefined();
		expect(addToProfile).toHaveBeenCalledExactlyOnceWith(bit, PROFILE);
		expect(harness.close).toHaveBeenCalledWith(false);
		expect(document.querySelector("[role='dialog']")).toBeNull();
	},
);

test("choosing another model updates automatic metadata but preserves fields the user edited", async () => {
	await mount();
	await chooseProvider("codex");
	await enter("discovered-model", FIRST.id);
	await enter("discovered-model", SECOND.id);
	expect(field("custom-model-name").value).toBe(SECOND.name);
	expect(field("custom-model-description").value).toBe(SECOND.description);
	expect(field("custom-model-context").value).toBe(
		String(SECOND.contextWindow),
	);
	await enter("custom-model-name", "My model name");
	await enter("custom-model-description", "My own description");
	await enter("custom-model-icon", "/custom/team-icon.svg");
	await enter("custom-model-tags", "team, private");
	await enter("custom-model-context", "64000");
	const vision = dialog().querySelector<HTMLButtonElement>(
		"#custom-model-vision",
	);
	if (!vision) throw new Error("Vision switch is missing");
	await click(vision);
	await enter("discovered-model", FIRST.id);
	await enter("discovered-model", SECOND.id);
	await click(button("Add model"));
	expect(savedBit().type).toBe("Vlm");
	expect(savedBit().meta.en).toMatchObject({
		name: "My model name",
		description: "My own description",
		icon: "/custom/team-icon.svg",
		tags: ["team", "private"],
	});
	expect(savedBit().parameters).toMatchObject({
		context_length: 64000,
		provider: { model_id: SECOND.id },
	});
});

test.each(["partial", "empty", "changed metadata", "failure"] as const)(
	"refresh with %s preserves the exact selected model and edited fields",
	async (state) => {
		await mount();
		await chooseProvider("claude-code");
		await enter("discovered-model", FIRST.id);
		await enter("custom-model-name", "Keep my Claude");
		if (state === "failure")
			discover.mockRejectedValueOnce(new Error("Refresh failed; try again."));
		else
			discover.mockResolvedValueOnce(
				catalog(
					state === "empty"
						? []
						: state === "partial"
							? [SECOND]
							: [
									{
										...FIRST,
										name: "Refreshed runtime name",
										contextWindow: 1_048_576,
									},
								],
				),
			);
		await click(button("Refresh models"));
		await settle();
		expect(discover).toHaveBeenCalledTimes(2);
		if (state === "failure")
			expect(dialog().textContent).toContain("Refresh failed; try again.");
		expect(field("discovered-model").value).toBe(FIRST.id);
		expect(modelOptions()).toContain(FIRST.id);
		expect(field("custom-model-name").value).toBe("Keep my Claude");
		expect(field("custom-model-context").value).toBe(
			String(FIRST.contextWindow),
		);
		await click(button("Add model"));
		expect(savedBit().parameters?.provider?.model_id).toBe(FIRST.id);
	},
);

test("a model without a reported context limit keeps an editable default and explains the unknown limit", async () => {
	discover.mockResolvedValueOnce(
		catalog([
			FIRST,
			{ id: "unknown-limit", name: "Runtime model without limits" },
		]),
	);
	await mount();
	await chooseProvider("codex");
	await enter("discovered-model", FIRST.id);
	expect(field("custom-model-context").value).toBe(String(FIRST.contextWindow));
	await enter("discovered-model", "unknown-limit");
	expect(field("custom-model-context").value).toBe("128000");
	expect(
		dialog()
			.querySelector("#custom-model-vision")
			?.getAttribute("aria-checked"),
	).toBe("false");
	expect(dialog().textContent).toMatch(
		/context.{0,70}(not reported|unknown)|(?:not report|unknown).{0,70}context/i,
	);
	await enter("custom-model-context", "32768");
	await click(button("Add model"));
	expect(savedBit().parameters?.context_length).toBe(32768);
});

test("switching providers clears prior metadata and ignores a late discovery response", async () => {
	const stale = deferred<ExternalModelCatalog>();
	await mount();
	await chooseProvider("claude-code");
	await enter("discovered-model", FIRST.id);
	await enter("custom-model-name", "My old Claude name");
	await enter("custom-model-context", "64000");
	discover.mockReturnValueOnce(stale.promise);
	await click(button("Refresh models"));
	await click(button("Back to provider selection"));
	discover.mockResolvedValueOnce(catalog([SECOND]));
	await chooseProvider("codex");
	expect(discover.mock.calls.map(([provider]) => provider)).toEqual([
		"claude-code",
		"claude-code",
		"codex",
	]);
	expect(field("custom-model-name").value).toBe("");
	expect(field("custom-model-description").value).toBe("");
	expect(field("custom-model-tags").value).toBe("");
	expect(field("custom-model-context").value).toBe("128000");
	await act(async () => stale.resolve(catalog([FIRST])));
	expect(modelOptions()).toEqual([SECOND.id]);
	await enter("discovered-model", SECOND.id);
	await click(button("Add model"));
	expect(savedBit().meta.en.name).toBe(SECOND.name);
	expect(savedBit().parameters).toMatchObject({
		context_length: SECOND.contextWindow,
		provider: { provider_name: "custom:codex", model_id: SECOND.id },
	});
});

test.each([
	{
		state: "unavailable",
		result: {
			available: false,
			authenticated: false,
			models: [],
			message: "Install the runtime, then refresh.",
		},
	},
	{
		state: "signed out",
		result: {
			available: true,
			authenticated: false,
			models: [],
			message: "Sign in with the runtime, then refresh.",
		},
	},
	{
		state: "failure",
		result: new Error("Runtime discovery failed. Try refreshing."),
	},
])(
	"$state discovery can be retried without inventing a selectable model",
	async ({ result }) => {
		if (result instanceof Error) discover.mockRejectedValueOnce(result);
		else discover.mockResolvedValueOnce(result);
		await mount();
		await chooseProvider("claude-code");
		expect(dialog().textContent).toContain(result.message);
		expect(modelOptions()).toEqual([]);
		expect(button("Add model").disabled).toBe(true);
		await click(button("Refresh models"));
		await settle();
		expect(modelOptions()).toEqual([FIRST.id, SECOND.id]);
		await enter("discovered-model", FIRST.id);
		await click(button("Add model"));
		expect(save).toHaveBeenCalledTimes(1);
	},
);

test("closing and reopening setup cannot apply a previous discovery result", async () => {
	const stale = deferred<ExternalModelCatalog>();
	const current = deferred<ExternalModelCatalog>();
	discover
		.mockReturnValueOnce(stale.promise)
		.mockReturnValueOnce(current.promise);
	await mount();
	await chooseProvider("claude-code");
	await click(button("Cancel"));
	await click(button("Reopen setup", host));
	await chooseProvider("claude-code");
	await act(async () => stale.resolve(catalog([FIRST])));
	expect(modelOptions()).not.toContain(FIRST.id);
	await act(async () => current.resolve(catalog([SECOND])));
	expect(modelOptions()).toEqual([SECOND.id]);
	expect(field("discovered-model").value).toBe("");
	await enter("discovered-model", SECOND.id);
	await click(button("Add model"));
	expect(savedBit().parameters?.provider?.model_id).toBe(SECOND.id);
});

test("native setup keeps manual model entry available when runtime discovery has no choices", async () => {
	discover.mockResolvedValueOnce(catalog([]));
	await mount();
	await chooseProvider("claude-code");
	await click(button("Enter a model ID manually"));
	await enter("custom-model-model_id", "my-private-model");
	await enter("custom-model-name", "Private Claude model");
	await click(button("Add model"));
	expect(savedBit().parameters?.provider).toMatchObject({
		provider_name: "custom:claude-code",
		model_id: "my-private-model",
	});
});

test("browser Codex uses the manual token path and keeps credentials out of the saved Bit", async () => {
	await mount(true);
	expect(dialog().textContent).not.toContain("Claude Code");
	await chooseProvider("codex");
	expect(discover).not.toHaveBeenCalled();
	expect(dialog().querySelector("#discovered-model")).toBeNull();
	await enter("custom-model-model_id", "account-model-id");
	await enter("custom-model-name", "My account model");
	expect(button("Add model").disabled).toBe(true);
	await enter("custom-model-access_token", "fixture-access-token");
	await click(button("Add model"));
	expect(savedBit().parameters?.provider).toMatchObject({
		provider_name: "custom:codex",
		model_id: "account-model-id",
	});
	expect(JSON.stringify(savedBit())).not.toContain("fixture-access-token");
	expect(save.mock.calls[0]?.[1]).toEqual({
		access_token: "fixture-access-token",
	});
	expect(discover).not.toHaveBeenCalled();
});

function existingModel(provider: ExternalModelProvider): IBit {
	const timestamp = { secs_since_epoch: 100, nanos_since_epoch: 0 };
	return {
		id: `saved-${provider}-bit`,
		type: provider === "codex" ? IBitTypes.Vlm : IBitTypes.Llm,
		meta: {
			en: {
				name: "My saved model",
				description: "Keep my saved description.",
				icon: "/custom/saved-icon.svg",
				tags: ["saved", "team"],
				preview_media: [],
				created_at: timestamp,
				updated_at: timestamp,
			},
		},
		parameters: {
			context_length: 96_000,
			custom_runtime_options: { trace: false },
			model_classification: { coding: 0.8 },
			provider: {
				provider_name: `custom:${provider}`,
				model_id: "saved-model-missing-from-catalog",
				transport_hint: "existing-transport",
				version: "saved-provider-version",
				params: {
					model_id: "saved-model-missing-from-catalog",
					deployment_tag: "existing-deployment",
					access_token: "stored-fixture-secret",
				},
			},
		},
		authors: ["owner"],
		dependencies: ["existing-dependency"],
		dependency_tree_hash: "dependency-tree-hash",
		hash: "existing-hash",
		hub: "",
		created: "2025-01-01T00:00:00Z",
		updated: "2025-01-01T00:00:00Z",
		custom_extension: { keep: true },
	};
}

test.each(["claude-code", "codex"] as const)(
	"editing a %s Bit preserves stored metadata and extensions across discovery and model changes",
	async (provider) => {
		const existing = existingModel(provider);
		await mount(false, existing);
		await settle();
		expect(discover).toHaveBeenCalledExactlyOnceWith(provider);
		expect(field("discovered-model").value).toBe(
			"saved-model-missing-from-catalog",
		);
		expect(modelOptions()).toContain("saved-model-missing-from-catalog");
		expect(field("custom-model-name").value).toBe(existing.meta.en.name);
		expect(field("custom-model-description").value).toBe(
			existing.meta.en.description,
		);
		expect(field("custom-model-context").value).toBe("96000");
		expect(field("custom-model-tags").value).toBe("saved, team");
		expect(field("custom-model-icon").value).toBe(existing.meta.en.icon);
		if (provider === "codex") {
			await click(button("Advanced options"));
			expect(field("custom-model-access_token").value).toBe("");
		}
		await enter("discovered-model", SECOND.id);
		expect(field("custom-model-name").value).toBe(existing.meta.en.name);
		expect(field("custom-model-context").value).toBe("96000");
		expect(
			dialog()
				.querySelector("#custom-model-vision")
				?.getAttribute("aria-checked"),
		).toBe(String(provider === "codex"));
		await click(button("Save changes"));
		const bit = savedBit();
		expect(bit).toMatchObject({
			id: existing.id,
			type: existing.type,
			dependencies: existing.dependencies,
			custom_extension: existing.custom_extension,
			meta: {
				en: {
					name: existing.meta.en.name,
					description: existing.meta.en.description,
					icon: existing.meta.en.icon,
					tags: existing.meta.en.tags,
				},
			},
			parameters: {
				context_length: 96_000,
				custom_runtime_options: { trace: false },
				model_classification: { coding: 0.8 },
				provider: {
					provider_name: `custom:${provider}`,
					model_id: SECOND.id,
					transport_hint: "existing-transport",
					version: "saved-provider-version",
					params: {
						model_id: SECOND.id,
						deployment_tag: "existing-deployment",
					},
				},
			},
		});
		expect(JSON.stringify(bit)).not.toContain("stored-fixture-secret");
		expect(save.mock.calls[0]?.[1]).toEqual({
			access_token: "stored-fixture-secret",
		});
		expect(addToProfile).not.toHaveBeenCalled();
	},
);

test("editing a Bit fills blank metadata from the chosen runtime model", async () => {
	const existing = existingModel("codex");
	existing.meta.en.name = "";
	existing.meta.en.description = "";
	existing.meta.en.icon = "";
	existing.meta.en.tags = [];
	existing.parameters.context_length = undefined;
	await mount(false, existing);
	await settle();
	await enter("discovered-model", FIRST.id);
	await click(button("Save changes"));
	expect(savedBit().meta.en).toMatchObject({
		name: FIRST.name,
		description: FIRST.description,
		icon: "/provider/gpt.webp",
		tags: ["codex", "chat", "reasoning", "vision"],
	});
	expect(savedBit().parameters.context_length).toBe(FIRST.contextWindow);
	expect(savedBit().id).toBe(existing.id);
	expect(addToProfile).not.toHaveBeenCalled();
});

test.each([
	{ provider: "claude-code", webMode: true },
	{ provider: "codex", webMode: true },
	{ provider: "openai", webMode: false },
] as const)(
	"$provider editing in webMode=$webMode retains the generic Bit editor",
	async ({ provider, webMode }) => {
		const existing = existingModel("codex");
		existing.parameters.provider.provider_name = `custom:${provider}`;
		await mount(webMode, existing);
		expect(
			host.querySelector("[data-testid='generic-bit-editor']")?.textContent,
		).toBe(existing.id);
		expect(discover).not.toHaveBeenCalled();
		expect(save).not.toHaveBeenCalled();
	},
);

test.each([false, true])(
	"manual model entry clears prior runtime metadata while preserving dirty fields=%s",
	async (edited) => {
		await mount();
		await chooseProvider("codex");
		await enter("discovered-model", FIRST.id);
		if (edited) {
			await enter("custom-model-name", "My manual name");
			await enter("custom-model-context", "48000");
		}
		await click(button("Enter a model ID manually"));
		await enter("custom-model-model_id", "my-unlisted-model");
		expect(field("custom-model-name").value).toBe(
			edited ? "My manual name" : "my-unlisted-model",
		);
		expect(field("custom-model-description").value).toBe(
			"my-unlisted-model via Codex.",
		);
		expect(field("custom-model-context").value).toBe(
			edited ? "48000" : "128000",
		);
		await click(button("Add model"));
		expect(savedBit().type).toBe("Llm");
		expect(savedBit().meta.en.tags).toEqual(["codex", "chat"]);
		expect(savedBit().parameters).toMatchObject({
			context_length: edited ? 48000 : 128000,
			provider: { model_id: "my-unlisted-model" },
		});
	},
);
