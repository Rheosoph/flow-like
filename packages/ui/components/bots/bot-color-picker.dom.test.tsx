// @vitest-environment happy-dom

import { act, useState } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BotColorPicker, type BotColorPickerProps } from "./bot-color-picker";
import { defaultBotCatalog } from "./catalog";
import type { BotAppearance } from "./types";

let root: Root;
let container: HTMLDivElement;
const shape = defaultBotCatalog.shapes.mochi;

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.unstubAllGlobals();
});

function button(name: string) {
	const element = [...container.querySelectorAll("button")].find(
		(item) =>
			(item.getAttribute("aria-label") ?? item.textContent?.trim()) === name,
	);
	if (!element) throw new Error(`Missing button: ${name}`);
	return element;
}

async function click(name: string) {
	await act(async () => button(name).click());
}

function input(name: string) {
	const element = container.querySelector<HTMLInputElement>(
		`input[aria-label="${name}"]`,
	);
	if (!element) throw new Error(`Missing input: ${name}`);
	return element;
}

async function type(name: string, value: string) {
	await act(async () => {
		const element = input(name);
		Object.getOwnPropertyDescriptor(
			HTMLInputElement.prototype,
			"value",
		)?.set?.call(element, value);
		element.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

async function mount(options: Partial<BotColorPickerProps> = {}) {
	const onChange = vi.fn();
	const onReset = vi.fn();
	function Harness() {
		const [value, setValue] = useState<BotAppearance["colors"]>(options.value);
		return (
			<BotColorPicker
				shape={shape}
				catalog={defaultBotCatalog}
				{...options}
				value={value}
				onChange={(slot, selection) => {
					onChange(slot, selection);
					setValue((previous) => {
						const next = { ...previous };
						if (selection) next[slot] = selection;
						else delete next[slot];
						return next;
					});
				}}
				onReset={() => {
					onReset();
					setValue({});
				}}
			/>
		);
	}
	await act(async () => root.render(<Harness />));
	return { onChange, onReset };
}

describe("BotColorPicker", () => {
	it("edits all seven roles and supports role and palette resets", async () => {
		const { onChange, onReset } = await mount();
		expect(
			container.querySelectorAll(".fl-bot-color-role-toggle"),
		).toHaveLength(7);
		await click("Choose Coral for Body");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "swatch",
			colorId: "color-coral",
		});
		await click("Edit Accent color");
		await click("Choose Leaf for Accent");
		expect(onChange).toHaveBeenLastCalledWith("accent", {
			kind: "swatch",
			colorId: "color-leaf",
		});
		await click("Reset Body color");
		expect(onChange).toHaveBeenLastCalledWith("body", undefined);
		await click("Reset colors");
		expect(onReset).toHaveBeenCalledTimes(1);
		expect(button("Reset colors").disabled).toBe(true);
	});

	it.each([undefined, []] as const)(
		"previews premium colors without an owned pack: %s",
		async (ownedPackIds) => {
			const onRequestPack = vi.fn();
			const { onChange } = await mount({ ownedPackIds, onRequestPack });
			await click("Pastels color pack");
			expect(button("Choose Soft mint for Body").disabled).toBe(false);
			expect(
				button("Choose Soft mint for Body").getAttribute("data-preview-only"),
			).toBe("true");
			await click("Choose Soft mint for Body");
			expect(onChange).toHaveBeenLastCalledWith("body", {
				kind: "swatch",
				colorId: "pastel-mint",
			});
			expect(container.textContent).toContain("Soft mint · Preview only");
			await click("View Pastels color pack");
			expect(onRequestPack).toHaveBeenCalledWith(
				defaultBotCatalog.colorPacks["colors-pastels"],
			);
			expect(onChange).toHaveBeenCalledTimes(1);
			await click("Color lab color pack");
			expect(input("Pick custom color for Body").disabled).toBe(false);
			await type("Hex color for Body", "#f80");
			await click("Apply custom color");
			expect(onChange).toHaveBeenLastCalledWith("body", {
				kind: "hex",
				value: "#FF8800",
				packId: "color-lab",
			});
		},
	);

	it("distinguishes owned colors while keeping other premium colors selectable", async () => {
		const { onChange } = await mount({ ownedPackIds: ["colors-pastels"] });
		await click("Pastels color pack");
		expect(button("Pastels color pack").textContent).toContain("Owned");
		expect(
			button("Choose Soft mint for Body").hasAttribute("data-preview-only"),
		).toBe(false);
		await click("Choose Soft mint for Body");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "swatch",
			colorId: "pastel-mint",
		});
		await click("Holographic color pack");
		expect(button("Holographic color pack").textContent).toContain("Premium");
		expect(button("Choose Aurora for Body").disabled).toBe(false);
		expect(
			button("Choose Aurora for Body").getAttribute("data-preview-only"),
		).toBe("true");
		await click("Choose Aurora for Body");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "swatch",
			colorId: "holo-aurora",
		});
	});

	it("keeps shared included colors usable inside an unowned premium pack", async () => {
		const shared = {
			id: "shared",
			label: "Shared colors",
			description: "Colors from several collections",
			colorIds: ["color-coral", "pastel-mint"],
			access: "premium" as const,
		};
		const { onChange } = await mount({
			catalog: {
				...defaultBotCatalog,
				colorPacks: { ...defaultBotCatalog.colorPacks, shared },
			},
		});
		await click("Shared colors color pack");
		expect(container.textContent).toContain(
			"Unlock the pack to save colors marked Preview only.",
		);
		expect(button("Choose Soft mint for Body").disabled).toBe(false);
		expect(
			button("Choose Soft mint for Body").getAttribute("data-preview-only"),
		).toBe("true");
		expect(
			button("Choose Coral for Body").hasAttribute("data-preview-only"),
		).toBe(false);
		await click("Choose Coral for Body");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "swatch",
			colorId: "color-coral",
		});
		await click("Choose Soft mint for Body");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "swatch",
			colorId: "pastel-mint",
		});
	});

	it("validates custom hex input before emitting a normalized color", async () => {
		const { onChange } = await mount();
		await click("Color lab color pack");
		await type("Hex color for Body", "not-a-color");
		expect(button("Apply custom color").disabled).toBe(true);
		expect(input("Hex color for Body").getAttribute("aria-invalid")).toBe(
			"true",
		);
		await act(async () =>
			input("Hex color for Body").dispatchEvent(
				new KeyboardEvent("keydown", {
					key: "Enter",
					bubbles: true,
					cancelable: true,
				}),
			),
		);
		expect(onChange).not.toHaveBeenCalled();
		await type("Hex color for Body", " #abc ");
		await click("Apply custom color");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "hex",
			value: "#AABBCC",
			packId: "color-lab",
		});
		expect(input("Hex color for Body").value).toBe("#AABBCC");
		await type("Pick custom color for Body", "#336699");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "hex",
			value: "#336699",
			packId: "color-lab",
		});
	});

	it("keeps custom hex editable after ownership is revoked without erasing saved colors", async () => {
		const onChange = vi.fn();
		const props: BotColorPickerProps = {
			shape,
			catalog: defaultBotCatalog,
			value: { body: { kind: "hex", value: "#112233", packId: "color-lab" } },
			ownedPackIds: ["color-lab"],
			onChange,
			onReset: vi.fn(),
		};
		await act(async () => root.render(<BotColorPicker {...props} />));
		await click("Color lab color pack");
		expect(input("Hex color for Body").value).toBe("#112233");
		await act(async () =>
			root.render(<BotColorPicker {...props} ownedPackIds={[]} />),
		);
		expect(input("Hex color for Body").value).toBe("#112233");
		expect(container.textContent).toContain("#112233 · Preview only");
		expect(onChange).not.toHaveBeenCalled();
		await type("Hex color for Body", "#def");
		await click("Apply custom color");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "hex",
			value: "#DDEEFF",
			packId: "color-lab",
		});
		await click("Reset Body color");
		expect(onChange).toHaveBeenCalledWith("body", undefined);
	});

	it("honors a catalog extension that deliberately includes custom hex access", async () => {
		const includedPack = {
			...defaultBotCatalog.colorPacks["color-lab"],
			id: "included-lab",
			label: "Included lab",
			access: "included" as const,
		};
		const { onChange } = await mount({
			catalog: {
				...defaultBotCatalog,
				colorPacks: {
					...defaultBotCatalog.colorPacks,
					[includedPack.id]: includedPack,
				},
			},
		});
		await click("Included lab color pack");
		await type("Hex color for Body", "#123");
		await click("Apply custom color");
		expect(onChange).toHaveBeenLastCalledWith("body", {
			kind: "hex",
			value: "#112233",
			packId: "included-lab",
		});
	});

	it("uses shape-defined roles and leaves reset available while effects disable selection", async () => {
		const { onChange } = await mount({
			shape: { ...shape, colorSlots: ["body", "ink"] },
			disabled: true,
			value: { body: { kind: "swatch", colorId: "color-coral" } },
		});
		expect(
			container.querySelectorAll(".fl-bot-color-role-toggle"),
		).toHaveLength(2);
		expect(button("Choose Leaf for Body").disabled).toBe(true);
		await click("Choose Leaf for Body");
		expect(onChange).not.toHaveBeenCalled();
		await click("Holographic color pack");
		expect(button("Choose Aurora for Body").disabled).toBe(true);
		await click("Choose Aurora for Body");
		await click("Color lab color pack");
		expect(container.querySelector('input[type="color"]')).toBeNull();
		expect(container.querySelector('input[type="text"]')).toBeNull();
		expect(onChange).not.toHaveBeenCalled();
		await click("Reset Body color");
		expect(onChange).toHaveBeenCalledWith("body", undefined);
	});

	it("reuses static paint definitions with unique SVG references", async () => {
		await mount({
			ownedPackIds: ["colors-holographic", "colors-patterns"],
			value: { body: { kind: "swatch", colorId: "holo-aurora" } },
		});
		await click("Holographic color pack");
		expect(container.querySelectorAll("linearGradient").length).toBeGreaterThan(
			1,
		);
		expect(container.querySelector('[data-paint-animated="true"]')).toBeNull();
		await click("Patterns color pack");
		expect(container.querySelectorAll("pattern").length).toBeGreaterThan(1);
		const ids = [...container.querySelectorAll("[id]")].map(
			(element) => element.id,
		);
		expect(new Set(ids).size).toBe(ids.length);
		for (const element of container.querySelectorAll("[fill^='url(#']")) {
			const paintId = element.getAttribute("fill")?.slice(5, -1);
			expect(ids).toContain(paintId);
		}
	});
});
