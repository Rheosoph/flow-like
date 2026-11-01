// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BotOutfitTile, type BotOutfitTileProps } from "./bot-outfit-tile";
import { defaultBotCatalog } from "./catalog";
import type { BotAppearance, BotAvatarProps } from "./types";

vi.mock("./bot-avatar", () => ({
	BotAvatar: ({ appearance }: BotAvatarProps) => (
		<span data-appearance={JSON.stringify(appearance)} />
	),
}));

let root: Root;
let container: HTMLDivElement;
let media: EventTarget & { matches: boolean };
let unmounted = false;

beforeEach(() => {
	vi.useFakeTimers();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	media = Object.assign(new EventTarget(), { matches: false });
	vi.stubGlobal(
		"matchMedia",
		vi.fn(() => media),
	);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	unmounted = false;
});

afterEach(async () => {
	if (!unmounted) await act(async () => root.unmount());
	container.remove();
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

const appearance: BotAppearance = Object.freeze({
	kind: "mochi",
	frame: "squircle",
	variant: "pastel",
	colors: Object.freeze({
		body: { kind: "swatch", colorId: "color-pink" },
		background: { kind: "swatch", colorId: "color-cream" },
		clothing: { kind: "swatch", colorId: "color-ink" },
		accent: { kind: "swatch", colorId: "color-lavender" },
	}) as BotAppearance["colors"],
});

async function mount(options: Partial<BotOutfitTileProps> = {}) {
	const onSelect = vi.fn();
	await act(async () =>
		root.render(
			<BotOutfitTile
				item={defaultBotCatalog.equipment.cardigan}
				appearance={appearance}
				catalog={defaultBotCatalog}
				expression="neutral"
				selected={false}
				locked={false}
				onSelect={onSelect}
				{...options}
			/>,
		),
	);
	return onSelect;
}

function button() {
	const element = container.querySelector("button");
	if (!element) throw new Error("Missing outfit tile");
	return element;
}

function preview(): BotAppearance {
	const value = container
		.querySelector("[data-appearance]")
		?.getAttribute("data-appearance");
	if (!value) throw new Error("Missing outfit preview");
	return JSON.parse(value);
}

async function pointer(type: string, pointerType = "mouse") {
	await act(async () =>
		button().dispatchEvent(
			new PointerEvent(type, { bubbles: true, pointerType }),
		),
	);
}

describe("BotOutfitTile", () => {
	it("cycles only outfit colors and restores the exact base after hover without editing the draft", async () => {
		const onSelect = await mount();
		const original = preview();
		expect(original.equipment).toEqual({ neck: "cardigan" });
		await pointer("pointerover");
		const first = preview();
		expect(first.colors?.clothing).not.toEqual(original.colors?.clothing);
		expect(first.colors?.body).toEqual(original.colors?.body);
		expect(first.colors?.background).toEqual(original.colors?.background);
		expect(first.variant).toBe(original.variant);
		await act(async () => vi.advanceTimersByTime(1400));
		expect(preview().colors?.clothing).not.toEqual(first.colors?.clothing);
		expect(onSelect).not.toHaveBeenCalled();
		await act(async () => button().click());
		expect(onSelect).toHaveBeenCalledExactlyOnceWith();
		await pointer("pointerout");
		expect(preview()).toEqual(original);
		expect(appearance.colors?.clothing).toEqual({
			kind: "swatch",
			colorId: "color-ink",
		});
		expect(vi.getTimerCount()).toBe(0);
	});

	it("previews on keyboard focus and stops on blur", async () => {
		await mount();
		const original = preview();
		await act(async () => button().focus());
		expect(button().hasAttribute("data-color-preview")).toBe(true);
		expect(preview()).not.toEqual(original);
		await act(async () => button().blur());
		expect(preview()).toEqual(original);
		expect(vi.getTimerCount()).toBe(0);
	});

	it("does not keep a mouse-clicked tile cycling after the pointer leaves", async () => {
		await mount();
		const original = preview();
		await pointer("pointerover");
		await pointer("pointerdown");
		await act(async () => button().focus());
		await pointer("pointerout");
		expect(document.activeElement).toBe(button());
		expect(preview()).toEqual(original);
		expect(vi.getTimerCount()).toBe(0);
	});

	it("does not start a hover preview for touch contact or resulting focus", async () => {
		await mount();
		const original = preview();
		await pointer("pointerover", "touch");
		await pointer("pointerdown", "touch");
		await act(async () => button().focus());
		expect(preview()).toEqual(original);
		expect(vi.getTimerCount()).toBe(0);
	});

	it("uses one still color idea for reduced motion and responds to preference changes", async () => {
		media.matches = true;
		await mount();
		const original = preview();
		await pointer("pointerover");
		const still = preview();
		expect(still).not.toEqual(original);
		await act(async () => vi.advanceTimersByTime(10000));
		expect(preview()).toEqual(still);
		expect(vi.getTimerCount()).toBe(0);
		await act(async () => {
			media.matches = false;
			media.dispatchEvent(new Event("change"));
		});
		await act(async () => vi.advanceTimersByTime(1400));
		expect(preview()).not.toEqual(still);
		await act(async () => {
			media.matches = true;
			media.dispatchEvent(new Event("change"));
		});
		expect(vi.getTimerCount()).toBe(0);
	});

	it("honors editable palette roles and leaves restricted accent colors alone", async () => {
		await mount({
			catalog: {
				...defaultBotCatalog,
				shapes: {
					...defaultBotCatalog.shapes,
					mochi: {
						...defaultBotCatalog.shapes.mochi,
						colorSlots: ["body", "clothing"],
					},
				},
			},
		});
		await pointer("pointerover");
		expect(preview().colors?.clothing).not.toEqual(appearance.colors?.clothing);
		expect(preview().colors?.accent).toEqual(appearance.colors?.accent);
	});

	it.each(["lcd", "blueprint"] as const)(
		"leaves %s previews under their effect's color control",
		async (effect) => {
			await mount({ appearance: { ...appearance, effect } });
			const original = preview();
			await pointer("pointerover");
			expect(preview()).toEqual(original);
			expect(
				container.querySelectorAll(".fl-bot-outfit-swatches > span"),
			).toHaveLength(0);
			expect(vi.getTimerCount()).toBe(0);
		},
	);

	it("omits unavailable or premium-only preview swatches in custom catalogs", async () => {
		await mount({ catalog: { ...defaultBotCatalog, colorPacks: {} } });
		const original = preview();
		await pointer("pointerover");
		expect(preview()).toEqual(original);
		expect(vi.getTimerCount()).toBe(0);
		await mount({ catalog: { ...defaultBotCatalog, colors: {} } });
		await pointer("pointerover");
		expect(preview()).toEqual(original);
		expect(vi.getTimerCount()).toBe(0);
	});

	it("cleans up the active interval and media listener on unmount", async () => {
		const removeListener = vi.spyOn(media, "removeEventListener");
		await mount();
		await pointer("pointerover");
		expect(vi.getTimerCount()).toBe(1);
		await act(async () => root.unmount());
		unmounted = true;
		expect(vi.getTimerCount()).toBe(0);
		expect(removeListener).toHaveBeenCalledWith("change", expect.any(Function));
	});
});
