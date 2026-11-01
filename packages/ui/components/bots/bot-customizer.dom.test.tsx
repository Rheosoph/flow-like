// @vitest-environment happy-dom

import { act, useState } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BotCustomizer, type BotCustomizerProps } from "./bot-customizer";
import { defaultBotCatalog } from "./catalog";
import { canSaveBotAppearance } from "./save-eligibility";
import type { BotAppearance, BotAvatarProps, BotCatalog } from "./types";

vi.mock("./bot-avatar", () => ({
	BotAvatar: ({
		appearance,
		label,
		animated,
		pose,
		expression,
		mood,
		agentState,
		decorative,
	}: BotAvatarProps) => (
		<div
			role={decorative ? undefined : "img"}
			aria-label={label}
			data-kind={appearance?.kind}
			data-animated={animated}
			data-pose={pose}
			data-expression={expression}
			data-mood={mood}
			data-agent-state={agentState}
			data-effect={appearance?.effect}
			data-emote={appearance?.emote}
			data-equipment={JSON.stringify(appearance?.equipment)}
		/>
	),
}));

const roundShape = {
	id: "round",
	label: "Round bot",
	series: "basics",
	description: "A gentle circle",
	palette: {
		body: "#aa88dd",
		background: "#eee8ff",
		ink: "#242233",
		cheek: "#ee88aa",
		clothing: "#663399",
		accent: "#ddaa22",
		sclera: "#fff",
	},
	anchors: {
		eyes: { left: [40, 60] as const, right: [60, 60] as const, radius: 8 },
		mouth: [50, 72] as const,
		head: { x: 50, y: 34, width: 40 },
		neck: { y: 80, bottom: 100 },
		hand: [25, 80] as const,
	},
	silhouette: "M20 40h60v60H20z",
};

const catalog: BotCatalog = {
	defaultKind: "round",
	colors: defaultBotCatalog.colors,
	colorPacks: defaultBotCatalog.colorPacks,
	packs: {
		"basics-crew": {
			id: "basics-crew",
			label: "Basics Crew",
			description: "Friendly shape companions",
			coverKind: "round",
			shapeIds: ["round"],
		},
		"tech-crew": {
			id: "tech-crew",
			label: "Tech Crew",
			description: "Automation companions",
			coverKind: "angular",
			shapeIds: ["angular"],
		},
	},
	series: {
		basics: { id: "basics", label: "Basics", description: "Friendly shapes" },
		tech: { id: "tech", label: "Tech", description: "Automation friends" },
	},
	shapes: {
		round: roundShape,
		angular: {
			...roundShape,
			id: "angular",
			label: "Angular bot",
			series: "tech",
			description: "A helpful microchip",
		},
		custom: {
			...roundShape,
			id: "custom",
			label: "Custom bot",
			series: undefined,
		},
	},
	equipment: {
		cap: {
			id: "cap",
			label: "Round cap",
			slot: "head",
			kinds: ["round"],
			render: () => null,
		},
		helm: {
			id: "helm",
			label: "Angular helmet",
			slot: "head",
			kinds: ["angular"],
			render: () => null,
		},
		scarf: { id: "scarf", label: "Scarf", slot: "neck", render: () => null },
		glasses: {
			id: "glasses",
			label: "Glasses",
			slot: "eyes",
			render: () => null,
		},
	},
	skills: {
		code: { id: "code", label: "Code", color: "#663399", glyph: "<>" },
		chat: { id: "chat", label: "Chat", color: "#226699", glyph: "…" },
	},
};

let root: Root;
let container: HTMLDivElement;

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

function button(name: string, scope: ParentNode = container) {
	const found = [...scope.querySelectorAll("button")].find(
		(element) =>
			(element.getAttribute("aria-label") ??
				element.textContent?.replace(/[+✓]/g, "").trim()) === name,
	);
	if (!found) throw new Error(`Missing button: ${name}`);
	return found;
}

function group(name: string) {
	const found = container.querySelector(`fieldset[aria-label="${name}"]`);
	if (!found) throw new Error(`Missing group: ${name}`);
	return found;
}

async function click(name: string, scope: ParentNode = container) {
	await act(async () => button(name, scope).click());
}

async function section(name: string) {
	const buttons = [
		...group("Customization sections").querySelectorAll("button"),
	];
	const found = buttons.find((element) => element.textContent?.endsWith(name));
	if (!found) throw new Error(`Missing section: ${name}`);
	await act(async () => found.click());
}

async function search(value: string, label = "Search crew") {
	const input = container.querySelector<HTMLInputElement>(
		`input[aria-label="${label}"]`,
	);
	if (!input) throw new Error("Missing search field");
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			HTMLInputElement.prototype,
			"value",
		)?.set?.call(input, value);
		input.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

async function mount(
	initial: BotAppearance,
	onChange = vi.fn(),
	options: Pick<
		BotCustomizerProps,
		| "catalog"
		| "ownedPackIds"
		| "onRequestPack"
		| "onRequestColorPack"
		| "onSave"
	> = {},
) {
	function Harness() {
		const [value, setValue] = useState(initial);
		return (
			<BotCustomizer
				value={value}
				catalog={catalog}
				{...options}
				onChange={(next) => {
					onChange(next);
					setValue(next);
				}}
			/>
		);
	}
	await act(async () => root.render(<Harness />));
	return onChange;
}

describe("BotCustomizer", () => {
	it("resets the complete draft and preview to the catalog default without saving", async () => {
		const initial: BotAppearance = {
			kind: "angular",
			equipment: { head: "helm", neck: "scarf" },
			colors: { body: { kind: "swatch", colorId: "color-sky" } },
			variant: "gold",
			frame: "square",
			backdrop: "stars",
			halo: "orbit",
			effect: "pixel",
			emote: "heart",
			dark: true,
			skills: ["code"],
		};
		const original = structuredClone(initial);
		const onSave = vi.fn();
		const onChange = await mount(initial, vi.fn(), { onSave });
		await search("angular");
		await click("Pause preview animation");
		await click("Happy", group("Preview expression"));
		await click("Follow cursor", group("Preview pose"));
		const mood = container.querySelector<HTMLSelectElement>(
			'[aria-label="Preview mood or status"]',
		);
		if (!mood) throw new Error("Missing mood picker");
		await act(async () => {
			mood.value = "state:offline";
			mood.dispatchEvent(new Event("change", { bubbles: true }));
		});
		await click("Reset bot");
		expect(onChange).toHaveBeenCalledExactlyOnceWith({ kind: "round" });
		expect(onSave).not.toHaveBeenCalled();
		expect(initial).toEqual(original);
		const preview = container.querySelector('[role="img"]');
		expect(preview?.getAttribute("data-kind")).toBe("round");
		expect(preview?.getAttribute("data-expression")).toBe("neutral");
		expect(preview?.getAttribute("data-pose")).toBe("idle");
		expect(preview?.getAttribute("data-animated")).toBe("true");
		expect(preview?.getAttribute("data-agent-state")).toBe("idle");
		expect(mood.value).toBe("custom");
		expect(
			container.querySelector<HTMLInputElement>('[aria-label="Search crew"]')
				?.value,
		).toBe("");
		expect(button("Choose Angular bot")).toBeDefined();
		expect(button("Choose Custom bot")).toBeDefined();
	});

	it("randomizes custom catalog items with compatible owned gear and no stale draft fields", async () => {
		const withGear: BotCatalog = {
			...catalog,
			packs: {
				...catalog.packs,
				"basics-crew": {
					...catalog.packs["basics-crew"],
					equipmentIds: ["cap", "scarf", "custom-hat"],
				},
				"tech-crew": {
					...catalog.packs["tech-crew"],
					equipmentIds: ["helm", "locked-coat"],
				},
			},
			equipment: {
				...catalog.equipment,
				"custom-hat": {
					id: "custom-hat",
					label: "Custom hat",
					slot: "head",
					kinds: ["custom"],
					render: () => null,
				},
				"locked-coat": {
					id: "locked-coat",
					label: "Locked coat",
					slot: "neck",
					kinds: ["custom"],
					render: () => null,
				},
			},
		};
		const initial: BotAppearance = {
			kind: "round",
			equipment: { head: "cap", neck: "locked-coat" },
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
			variant: "gold",
			dark: true,
			skills: ["code"],
		};
		const original = structuredClone(initial);
		const ownedPackIds = ["basics-crew"];
		const onSave = vi.fn();
		const onChange = await mount(initial, vi.fn(), {
			catalog: withGear,
			ownedPackIds,
			onSave,
		});
		await click("Happy", group("Preview expression"));
		const random = vi.spyOn(Math, "random").mockReturnValue(0.9);
		try {
			await click("Randomize bot");
		} finally {
			random.mockRestore();
		}
		expect(onChange).toHaveBeenCalledTimes(1);
		const next = onChange.mock.calls[0][0] as BotAppearance;
		expect(next.kind).toBe("custom");
		expect(next.equipment?.neck).toBe("scarf");
		expect(next.equipment?.head).toBe("custom-hat");
		expect(next.colors).toBeUndefined();
		expect(next.variant).toBeUndefined();
		expect(next.dark).toBeUndefined();
		expect(next.skills).toBeUndefined();
		expect(canSaveBotAppearance(next, withGear, ownedPackIds)).toBe(true);
		expect(button("Save bot").disabled).toBe(false);
		expect(onSave).not.toHaveBeenCalled();
		expect(initial).toEqual(original);
		expect(JSON.parse(JSON.stringify(next))).toEqual(next);
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-expression"),
		).toBe("neutral");
	});

	it("groups every look and changes effects without clearing the outfit or emote", async () => {
		const value: BotAppearance = {
			kind: "round",
			equipment: { head: "cap", neck: "scarf" },
			emote: "heart",
		};
		const onChange = await mount(value);
		await section("Finish");
		await click("Effect", group("Finish categories"));
		const groups = [...container.querySelectorAll("[data-effect-group]")];
		expect(groups.map((entry) => entry.getAttribute("aria-label"))).toEqual([
			"Retro & Screen effects",
			"Print & Paper effects",
			"Pop effects",
			"Status effects",
		]);
		expect(
			groups.map((entry) => entry.querySelectorAll("button").length),
		).toEqual([5, 5, 3, 2]);
		for (const entry of groups)
			for (const avatar of entry.querySelectorAll("[data-kind]")) {
				expect(avatar.getAttribute("data-emote")).toBe("heart");
				expect(
					JSON.parse(avatar.getAttribute("data-equipment") ?? "null"),
				).toEqual(value.equipment);
			}
		for (const [label, effect] of [
			["CRT", "crt"],
			["Thermal", "thermal"],
			["Comic", "comic"],
			["Watercolor", "watercolor"],
			["Sticker", "sticker"],
			["Vinyl", "vinyl"],
			["Neon", "neon"],
			["None", "none"],
		]) {
			await click(`Choose ${label}`);
			expect(onChange).toHaveBeenLastCalledWith({ ...value, effect });
			expect(button(`Choose ${label}`).getAttribute("aria-pressed")).toBe(
				"true",
			);
		}
	});
	it("shows themed outfit sections without truncating or duplicating items", async () => {
		await mount({ kind: "mochi" }, vi.fn(), { catalog: defaultBotCatalog });
		await section("Outfit");
		await click("Clothing", group("Equipment slots"));
		const groups = [...container.querySelectorAll("[data-outfit-group]")];
		expect(groups).toHaveLength(24);
		expect(groups[0].getAttribute("aria-label")).toBe("Essentials outfits");
		const ids = groups.flatMap((group) =>
			[...group.querySelectorAll('[aria-label^="Choose "]')].map((item) =>
				item.getAttribute("aria-label"),
			),
		);
		expect(ids).toHaveLength(
			Object.values(defaultBotCatalog.equipment).filter(
				(item) => item.slot === "neck",
			).length,
		);
		expect(new Set(ids).size).toBe(ids.length);
		const basics = container.querySelector('[data-outfit-group="basics-crew"]');
		const basicsClothes = (
			defaultBotCatalog.packs["basics-crew"].equipmentIds ?? []
		).filter((id) => defaultBotCatalog.equipment[id].slot === "neck");
		expect(basicsClothes.length).toBeGreaterThan(6);
		expect(basics?.querySelectorAll('[aria-label^="Choose "]')).toHaveLength(
			basicsClothes.length,
		);
		await search("pirate", "Search outfits");
		expect(container.querySelectorAll("[data-outfit-group]")).toHaveLength(1);
		expect(
			container.querySelector('[data-outfit-group="pirates-crew"]'),
		).not.toBeNull();
	});

	it("shows shared equipment once and keeps it available in each containing theme", async () => {
		const shared = {
			...catalog,
			packs: {
				first: {
					...catalog.packs["basics-crew"],
					id: "first",
					label: "First",
					equipmentIds: ["cap"],
				},
				second: {
					...catalog.packs["tech-crew"],
					id: "second",
					label: "Second",
					equipmentIds: ["cap"],
				},
			},
		};
		await mount({ kind: "round" }, vi.fn(), {
			catalog: shared,
			ownedPackIds: ["second"],
		});
		await section("Outfit");
		expect(
			container.querySelectorAll('[aria-label="Choose Round cap"]'),
		).toHaveLength(1);
		expect(button("Choose Round cap").hasAttribute("data-locked")).toBe(false);
		const second = [
			...group("Outfit collections").querySelectorAll("button"),
		].find((button) => button.textContent?.startsWith("Second"));
		if (!second) throw new Error("Missing second collection");
		await act(async () => second.click());
		expect(container.querySelectorAll("[data-outfit-group]")).toHaveLength(1);
		expect(
			container.querySelector(
				'[data-outfit-group="second"] [aria-label="Choose Round cap"]',
			),
		).not.toBeNull();
	});

	it("filters outfits by collection and name without changing the equipped item", async () => {
		const onChange = await mount(
			{ kind: "mochi", equipment: { head: "crown" } },
			vi.fn(),
			{ catalog: defaultBotCatalog },
		);
		await section("Outfit");
		await search("on duty", "Search outfits");
		expect(button("Choose Police cap")).toBeTruthy();
		expect(container.querySelector('[aria-label="Choose Crown"]')).toBeNull();
		expect(onChange).not.toHaveBeenCalled();
		await search("", "Search outfits");
		await click("Essentials", group("Outfit collections"));
		expect(button("Choose Crown").getAttribute("aria-pressed")).toBe("true");
		expect(
			container.querySelector('[aria-label="Choose Police cap"]'),
		).toBeNull();
		await search("nothing matches this", "Search outfits");
		expect(container.textContent).toContain("No items match these filters");
		await click("Choose None");
		expect(onChange).toHaveBeenLastCalledWith({ kind: "mochi", equipment: {} });
		await click("Clear outfit filters");
		expect(button("Choose Police cap")).toBeTruthy();
		expect(button("Choose Crown")).toBeTruthy();
	});

	it("keeps a selected outfit collection while switching slots and shows every item in it", async () => {
		await mount({ kind: "mochi" }, vi.fn(), { catalog: defaultBotCatalog });
		await section("Outfit");
		const collectionButton = [
			...group("Outfit collections").querySelectorAll("button"),
		].find((button) => button.textContent?.startsWith("On duty"));
		if (!collectionButton) throw new Error("Missing On duty collection");
		await act(async () => collectionButton.click());
		await click("Clothing", group("Equipment slots"));
		const clothing = (
			defaultBotCatalog.packs["service-crew"].equipmentIds ?? []
		).filter((id) => defaultBotCatalog.equipment[id].slot === "neck");
		for (const id of clothing)
			expect(
				button(`Choose ${defaultBotCatalog.equipment[id].label}`),
			).toBeTruthy();
		expect(
			container.querySelectorAll(
				'.fl-bot-studio-panel [aria-label^="Choose "]',
			),
		).toHaveLength(clothing.length + 1);
		expect(container.querySelector('[aria-label="Choose Sweater"]')).toBeNull();
	});

	it("groups the default crew into twenty-three distinct packs without inventing ownership", async () => {
		await mount({ kind: defaultBotCatalog.defaultKind }, vi.fn(), {
			catalog: defaultBotCatalog,
		});
		expect(container.querySelectorAll(".fl-bot-studio-pack")).toHaveLength(23);
		for (const pack of Object.values(defaultBotCatalog.packs)) {
			const panel = [...container.querySelectorAll(".fl-bot-studio-pack")].find(
				(element) =>
					element.getAttribute("aria-label") === `${pack.label} pack`,
			);
			expect(panel?.textContent).toContain(pack.description);
			expect(panel?.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(
				pack.shapeIds.length,
			);
		}
		expect(
			container.querySelectorAll(".fl-bot-studio-pack-status"),
		).toHaveLength(0);
		expect(container.querySelectorAll(".fl-bot-studio-pack-link")).toHaveLength(
			0,
		);
		expect(
			container.querySelectorAll(".fl-bot-studio-tile:disabled"),
		).toHaveLength(0);
	});

	it("previews unowned characters and blocks only saving until an available character is chosen", async () => {
		const onRequestPack = vi.fn();
		const onSave = vi.fn();
		const onChange = await mount({ kind: "custom" }, vi.fn(), {
			ownedPackIds: ["basics-crew"],
			onRequestPack,
			onSave,
		});
		expect(button("Choose Angular bot").disabled).toBe(false);
		await click("Choose Angular bot");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "angular",
			equipment: {},
		});
		expect(button("Save bot").disabled).toBe(true);
		await click("Save bot");
		expect(onSave).not.toHaveBeenCalled();
		await click("View Tech Crew pack");
		expect(onRequestPack).toHaveBeenCalledWith(catalog.packs["tech-crew"]);
		expect(onChange).toHaveBeenCalledTimes(1);
		await click("Choose Round bot");
		expect(onChange).toHaveBeenLastCalledWith({ kind: "round", equipment: {} });
		expect(button("Save bot").disabled).toBe(false);
		await click("Save bot");
		expect(onSave).toHaveBeenCalledWith({ kind: "round", equipment: {} });
		expect(
			[...container.querySelectorAll(".fl-bot-studio-pack-status")].map(
				(element) => element.textContent,
			),
		).toEqual(["Owned", "Preview"]);
	});

	it("carries and previews compatible unowned gear while saving stays blocked", async () => {
		const paidGear = {
			id: "gear",
			label: "Tailor",
			description: "Clothing pack",
			coverKind: "round",
			shapeIds: ["round"],
			equipmentIds: ["scarf"],
		};
		const withGear = {
			...catalog,
			packs: { ...catalog.packs, gear: paidGear },
		};
		const onRequestPack = vi.fn();
		const onSave = vi.fn();
		const onChange = await mount(
			{ kind: "round", equipment: { neck: "scarf" } },
			vi.fn(),
			{
				catalog: withGear,
				ownedPackIds: ["basics-crew", "tech-crew"],
				onRequestPack,
				onSave,
			},
		);
		await click("Choose Angular bot");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "angular",
			equipment: { neck: "scarf" },
		});
		expect(button("Save bot").disabled).toBe(true);
		await click("Edit clothing: Scarf");
		expect(button("Choose Scarf").disabled).toBe(false);
		await click("Choose None");
		expect(button("Save bot").disabled).toBe(false);
		await click("Choose Scarf");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "angular",
			equipment: { neck: "scarf" },
		});
		expect(button("Save bot").disabled).toBe(true);
		await click("Save bot");
		expect(onSave).not.toHaveBeenCalled();
		await click("View Tailor pack ↗");
		expect(onRequestPack).toHaveBeenLastCalledWith(paidGear);
	});

	it("requests a pack with empty ownership without changing the selected appearance", async () => {
		const onRequestPack = vi.fn();
		const onChange = await mount({ kind: "custom", halo: "flow" }, vi.fn(), {
			ownedPackIds: [],
			onRequestPack,
		});
		await click("View Basics Crew pack");
		expect(onRequestPack).toHaveBeenCalledWith(catalog.packs["basics-crew"]);
		expect(onChange).not.toHaveBeenCalled();
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-kind"),
		).toBe("custom");
		expect(button("Choose Round bot").disabled).toBe(false);
	});

	it("keeps supplied unowned characters visible and allows previewing other characters", async () => {
		const onChange = await mount(
			{ kind: "angular", variant: "gold" },
			vi.fn(),
			{ ownedPackIds: [] },
		);
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-kind"),
		).toBe("angular");
		expect(container.textContent).toContain(
			"Unlock required packs to save this bot.",
		);
		expect(button("Choose Round bot").disabled).toBe(false);
		await click("Choose Round bot");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			variant: "gold",
			equipment: {},
		});
		await click("Choose Custom bot");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "custom",
			variant: "gold",
			equipment: {},
		});
	});

	it("previews premium colors and opens the required color pack without saving", async () => {
		const onSave = vi.fn();
		const onRequestColorPack = vi.fn();
		const onChange = await mount({ kind: "round" }, vi.fn(), {
			onSave,
			onRequestColorPack,
		});
		await section("Colors");
		await click("Holographic color pack");
		await click("Choose Aurora for Body");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			variant: "original",
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		});
		expect(button("Save bot").disabled).toBe(true);
		await click("Save bot");
		expect(onSave).not.toHaveBeenCalled();
		const requirements = container.querySelector(
			'[aria-label="Save requirements"]',
		);
		expect(requirements?.textContent).toContain("Aurora");
		await click("Holographic", requirements ?? container);
		expect(onRequestColorPack).toHaveBeenCalledWith(
			catalog.colorPacks["colors-holographic"],
		);
		expect(onChange).toHaveBeenCalledTimes(1);
		await click("Reset Body color");
		expect(button("Save bot").disabled).toBe(false);
	});

	it("lists alternative packs with or and saves when either pack is owned", async () => {
		const bundle = {
			id: "bundle",
			label: "Mixed Bundle",
			description: "A mixed collection",
			coverKind: "round",
			shapeIds: ["round", "angular"],
		};
		const extended = { ...catalog, packs: { ...catalog.packs, bundle } };
		const value = { kind: "angular" };
		const onSave = vi.fn();
		await act(async () =>
			root.render(
				<BotCustomizer
					value={value}
					catalog={extended}
					ownedPackIds={[]}
					onChange={vi.fn()}
					onSave={onSave}
				/>,
			),
		);
		const requirements = container.querySelector(
			'[aria-label="Save requirements"]',
		);
		expect(requirements?.textContent).toContain("Tech CreworMixed Bundle");
		expect(button("Save bot").disabled).toBe(true);
		await act(async () =>
			root.render(
				<BotCustomizer
					value={value}
					catalog={extended}
					ownedPackIds={["bundle"]}
					onChange={vi.fn()}
					onSave={onSave}
				/>,
			),
		);
		expect(button("Save bot").disabled).toBe(false);
		await click("Save bot");
		expect(onSave).toHaveBeenCalledWith(value);
		await act(async () =>
			root.render(
				<BotCustomizer
					value={value}
					catalog={extended}
					ownedPackIds={[]}
					onChange={vi.fn()}
					onSave={onSave}
				/>,
			),
		);
		await click("Save bot");
		expect(onSave).toHaveBeenCalledTimes(1);
	});

	it("saves one detached snapshot while newer preview edits remain editable", async () => {
		let finishSave: () => void = () => {};
		const pending = new Promise<void>((resolve) => {
			finishSave = resolve;
		});
		const onSave = vi.fn((_snapshot: BotAppearance) => pending);
		const initial: BotAppearance = {
			kind: "round",
			equipment: { neck: "scarf" },
		};
		const onChange = await mount(initial, vi.fn(), { onSave });
		await act(async () => {
			button("Save bot").click();
			button("Save bot").click();
		});
		expect(onSave).toHaveBeenCalledTimes(1);
		const snapshot = onSave.mock.calls[0][0];
		expect(snapshot).toEqual(initial);
		expect(snapshot).not.toBe(initial);
		expect(snapshot.equipment).not.toBe(initial.equipment);
		expect(button("Save bot").disabled).toBe(true);
		await click("Choose Angular bot");
		expect(onChange.mock.calls.at(-1)?.[0].kind).toBe("angular");
		expect(snapshot.kind).toBe("round");
		expect(container.textContent).toContain(
			"Your latest edits are not part of this save.",
		);
		await act(async () => finishSave());
		expect(button("Save bot").disabled).toBe(false);
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-kind"),
		).toBe("angular");
		expect(onChange).toHaveBeenCalledTimes(1);
		expect(container.textContent).not.toContain(
			"Your latest edits are not part of this save.",
		);
	});

	it("reports an asynchronous failure against its original draft and permits retry", async () => {
		let failSave: (reason: Error) => void = () => {};
		const pending = new Promise<void>((_, reject) => {
			failSave = reject;
		});
		const onSave = vi
			.fn<NonNullable<BotCustomizerProps["onSave"]>>()
			.mockImplementationOnce(() => pending)
			.mockResolvedValue(undefined);
		await mount({ kind: "round" }, vi.fn(), { onSave });
		await click("Save bot");
		await click("Choose Angular bot");
		await act(async () => failSave(new Error("Storage failed")));
		expect(container.querySelector('[role="alert"]')?.textContent).toBe(
			"The previous draft could not be saved. Save your latest changes to try again.",
		);
		expect(button("Save bot").disabled).toBe(false);
		await click("Save bot");
		expect(onSave).toHaveBeenLastCalledWith({ kind: "angular", equipment: {} });
		expect(container.querySelector('[role="alert"]')).toBeNull();
	});

	it("reports a synchronous save failure without leaving the creator busy", async () => {
		await mount({ kind: "round" }, vi.fn(), {
			onSave: () => {
				throw new Error("Storage failed");
			},
		});
		await click("Save bot");
		expect(container.querySelector('[role="alert"]')?.textContent).toBe(
			"Could not save this bot. Try again.",
		);
		expect(button("Save bot").disabled).toBe(false);
	});

	it("repairs unavailable fields and a missing shape without removing valid premium previews", async () => {
		const initial = {
			kind: "missing",
			equipment: { head: "retired", neck: "scarf", feet: "unknown" },
			colors: {
				body: { kind: "swatch", colorId: "holo-aurora" },
				accent: { kind: "swatch", colorId: "retired" },
				feet: { kind: "swatch", colorId: "color-coral" },
			},
		} as unknown as BotAppearance;
		const paidGear = {
			id: "gear",
			label: "Tailor",
			description: "Clothes",
			coverKind: "round",
			shapeIds: [],
			equipmentIds: ["scarf"],
		};
		const onSave = vi.fn();
		const onChange = await mount(initial, vi.fn(), {
			catalog: { ...catalog, packs: { ...catalog.packs, gear: paidGear } },
			ownedPackIds: ["basics-crew"],
			onSave,
		});
		expect(button("Save bot").disabled).toBe(true);
		await click("Remove unavailable items");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			equipment: { neck: "scarf" },
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		});
		expect(button("Save bot").disabled).toBe(true);
		expect(
			container.querySelector('[aria-label="Save requirements"]')?.textContent,
		).toContain("Tailor");
		expect(
			container.querySelector('[aria-label="Save requirements"]')?.textContent,
		).toContain("Holographic");
		expect(container.textContent).not.toContain("Remove unavailable items");
		expect(initial.kind).toBe("missing");
	});

	it("repairs malformed containers and shows only the relevant recovery instruction", async () => {
		const initial = {
			kind: "round",
			equipment: [],
			colors: "invalid",
		} as unknown as BotAppearance;
		const onChange = await mount(initial, vi.fn(), { onSave: vi.fn() });
		expect(container.textContent).toContain(
			"Remove unavailable items to save this bot.",
		);
		expect(container.textContent).not.toContain("Unlock required packs");
		await click("Remove unavailable items");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			equipment: {},
			colors: {},
		});
		expect(button("Save bot").disabled).toBe(false);
	});

	it("deduplicates search results across overlapping packs", async () => {
		const bundle = {
			id: "bundle",
			label: "Mixed Bundle",
			description: "A mixed collection",
			coverKind: "round",
			shapeIds: ["round", "angular"],
		};
		await mount({ kind: "round" }, vi.fn(), {
			catalog: { ...catalog, packs: { ...catalog.packs, bundle } },
			ownedPackIds: ["bundle"],
		});
		await search("mixed");
		expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(2);
		expect(button("Choose Round bot").disabled).toBe(false);
		expect(button("Choose Angular bot").disabled).toBe(false);
	});
	it("filters the crew by series and search, including ungrouped extensions", async () => {
		const onChange = await mount({ kind: "round" });
		expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(3);
		await click("Tech1", group("Character series"));
		expect(container.textContent).toContain("Your bot is still Round bot.");
		expect(button("Show selected")).toBeTruthy();
		expect(onChange).not.toHaveBeenCalled();
		expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(1);
		expect(button("Choose Angular bot")).toBeTruthy();
		await search("microchip");
		expect(button("Choose Angular bot")).toBeTruthy();
		await search("cat");
		expect(container.textContent).toContain("No characters match your search.");
		await click("Clear filters");
		expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(3);
		await click("Other", group("Character series"));
		expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(1);
		expect(button("Choose Custom bot")).toBeTruthy();
	});

	it("clears incompatible equipment when changing shape and preserves universal items", async () => {
		const initial: BotAppearance = {
			kind: "round",
			equipment: { head: "cap", neck: "scarf", eyes: "glasses" },
			variant: "gold",
		};
		const onChange = await mount(initial);
		await click("Choose Angular bot");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "angular",
			equipment: { neck: "scarf", eyes: "glasses" },
			variant: "gold",
		});
		expect(initial.equipment?.head).toBe("cap");
		await section("Outfit");
		expect(button("Choose None").getAttribute("aria-pressed")).toBe("true");
		expect(button("Choose Angular helmet")).toBeTruthy();
		expect(
			container.querySelector('button[aria-label="Choose Round cap"]'),
		).toBeNull();
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-kind"),
		).toBe("angular");
	});

	it.each(["all", "other"] as const)(
		"treats an extension series named %s as its own filter",
		async (seriesId) => {
			const extendedCatalog: BotCatalog = {
				...catalog,
				packs: {
					...catalog.packs,
					all: {
						id: "all",
						label: "Star Pack",
						description: "Stars",
						coverKind: "star",
						shapeIds: ["star"],
					},
					other: {
						id: "other",
						label: "Planet Pack",
						description: "Planets",
						coverKind: "planet",
						shapeIds: ["planet"],
					},
				},
				series: {
					...catalog.series,
					all: { id: "all", label: "All stars", description: "Custom stars" },
					other: {
						id: "other",
						label: "Otherworld",
						description: "Custom planets",
					},
				},
				shapes: {
					...catalog.shapes,
					star: {
						...roundShape,
						id: "star",
						label: "Custom star",
						series: "all",
					},
					planet: {
						...roundShape,
						id: "planet",
						label: "Custom planet",
						series: "other",
					},
				},
			};
			await act(async () =>
				root.render(
					<BotCustomizer
						value={{ kind: "round" }}
						catalog={extendedCatalog}
						onChange={vi.fn()}
					/>,
				),
			);
			await click(
				seriesId === "all" ? "All stars1" : "Otherworld1",
				group("Character series"),
			);
			expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(1);
			expect(
				button(
					seriesId === "all" ? "Choose Custom star" : "Choose Custom planet",
				),
			).toBeTruthy();
			await click(
				seriesId === "all" ? "Star Pack1" : "Planet Pack1",
				group("Character packs"),
			);
			expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(1);
			expect(
				button(
					seriesId === "all" ? "Choose Custom star" : "Choose Custom planet",
				),
			).toBeTruthy();
			expect(
				group("Character packs").querySelectorAll('[aria-pressed="true"]'),
			).toHaveLength(1);
			await click("All5", group("Character series"));
			expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(5);
			await click("Other", group("Character series"));
			expect(container.querySelectorAll(".fl-bot-studio-tile")).toHaveLength(1);
			expect(button("Choose Custom bot")).toBeTruthy();
		},
	);

	it("opens an outfit slot from the preview, equips and removes items, and resets the outfit", async () => {
		const onChange = await mount({
			kind: "round",
			equipment: { neck: "scarf" },
		});
		await click("Edit headwear: None");
		await click("Choose Round cap");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			equipment: { neck: "scarf", head: "cap" },
		});
		await click("Choose None");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			equipment: { neck: "scarf" },
		});
		await click("Reset outfit");
		expect(onChange).toHaveBeenLastCalledWith({ kind: "round", equipment: {} });
		expect(button("Reset outfit").disabled).toBe(true);
	});

	it("previews and applies finish choices without changing the outfit", async () => {
		const onChange = await mount({
			kind: "round",
			equipment: { neck: "scarf" },
			variant: "gold",
		});
		await section("Finish");
		await click("Pattern", group("Finish categories"));
		await click("Choose Stripes");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			equipment: { neck: "scarf" },
			variant: "gold",
			backdrop: "stripes",
		});
		await click("Frame", group("Finish categories"));
		await click("Choose Square");
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			equipment: { neck: "scarf" },
			variant: "gold",
			backdrop: "stripes",
			frame: "square",
		});
	});

	it.each(["lcd", "blueprint"] as const)(
		"explains the %s palette and removes its effect without resetting the outfit",
		async (effect) => {
			const initial: BotAppearance = {
				kind: "round",
				effect,
				variant: "gold",
				equipment: { neck: "scarf" },
				halo: "flow",
				skills: ["code"],
			};
			const onChange = await mount(initial);
			await section("Colors");
			expect(container.textContent).toContain("controls this palette");
			expect(
				container.querySelector('button[aria-label="Choose Gold"]'),
			).toBeNull();
			await click("Remove color effect");
			expect(onChange).toHaveBeenLastCalledWith({ ...initial, effect: "none" });
			expect(button("Reset legacy finish")).toBeTruthy();
		},
	);

	it("toggles skills without duplicates and emits a serializable appearance", async () => {
		const onChange = await mount({
			kind: "round",
			skills: ["code"],
			halo: "flow",
		});
		await section("Skills");
		const chat = [
			...container.querySelectorAll(".fl-bot-studio-skills button"),
		].find((element) =>
			element.textContent?.includes("Chat"),
		) as HTMLButtonElement;
		await act(async () => chat.click());
		expect(onChange).toHaveBeenLastCalledWith({
			kind: "round",
			skills: ["code", "chat"],
			halo: "flow",
		});
		await act(async () => chat.click());
		const emitted = onChange.mock.calls.at(-1)?.[0];
		expect(emitted).toEqual({ kind: "round", skills: ["code"], halo: "flow" });
		expect(JSON.parse(JSON.stringify(emitted))).toEqual(emitted);
	});

	it("lets unrelated edits continue when saved colors contain malformed selections", async () => {
		const initial = {
			kind: "round",
			colors: { body: { kind: "bogus" } },
		} as unknown as BotAppearance;
		const onChange = await mount(initial);
		await section("Finish");
		await click("Frame", group("Finish categories"));
		await click("Choose Square");
		expect(onChange).toHaveBeenLastCalledWith({ ...initial, frame: "square" });
	});

	it("preserves unowned colors on edits and when choosing another compatible shape", async () => {
		const initial: BotAppearance = {
			kind: "round",
			colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
		};
		const onChange = await mount(initial);
		await section("Colors");
		await click("Edit Background color");
		await click("Essentials color pack");
		await click("Choose Cream for Background");
		expect(onChange.mock.calls.at(-1)?.[0].colors).toEqual({
			body: initial.colors?.body,
			background: { kind: "swatch", colorId: "color-cream" },
		});
		await section("Crew");
		await click("Choose Angular bot");
		expect(onChange.mock.calls.at(-1)?.[0].colors).toEqual({
			body: initial.colors?.body,
			background: { kind: "swatch", colorId: "color-cream" },
		});
	});

	it("only carries colors supported by the next shape", async () => {
		const onChange = await mount(
			{
				kind: "round",
				colors: {
					body: { kind: "swatch", colorId: "color-coral" },
					background: { kind: "swatch", colorId: "color-cream" },
				},
			},
			vi.fn(),
			{
				catalog: {
					...catalog,
					shapes: {
						...catalog.shapes,
						angular: { ...catalog.shapes.angular, colorSlots: ["body"] },
					},
				},
			},
		);
		await click("Choose Angular bot");
		expect(onChange.mock.calls.at(-1)?.[0].colors).toEqual({
			body: { kind: "swatch", colorId: "color-coral" },
		});
		await section("Colors");
		expect(
			container.querySelectorAll('[aria-label^="Edit "][aria-label$=" color"]'),
		).toHaveLength(1);
	});

	it("keeps expression and playback local to the preview", async () => {
		const onChange = await mount({ kind: "round" });
		await click("Pause preview animation");
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-animated"),
		).toBe("false");
		await click("Happy", group("Preview expression"));
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-expression"),
		).toBe("happy");
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-pose"),
		).toBe("idle");
		await click("Follow cursor", group("Preview pose"));
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-pose"),
		).toBe("track");
		for (const label of [
			"Neutral",
			"Happy",
			"Concerned",
			"Surprised",
			"Curious",
			"Sad",
			"Wink",
			"Love",
			"Angry",
			"Focused",
			"Oops",
		]) {
			await click(label, group("Preview expression"));
			expect(
				button(label, group("Preview expression")).getAttribute("aria-pressed"),
			).toBe("true");
			for (const avatar of container.querySelectorAll("[data-kind]")) {
				expect(avatar.getAttribute("data-expression")).toBe(
					label.toLowerCase(),
				);
			}
		}
		expect(onChange).not.toHaveBeenCalled();
	});

	it("previews moods and statuses without putting them into the saved outfit", async () => {
		const onChange = vi.fn();
		const onSave = vi.fn();
		const value = { kind: "round", equipment: { head: "cap" } };
		await mount(value, onChange, { onSave });
		const select = container.querySelector<HTMLSelectElement>(
			'[aria-label="Preview mood or status"]',
		);
		if (!select) throw new Error("Missing mood and status picker");
		expect(
			select.querySelectorAll('optgroup[label="Moods"] option'),
		).toHaveLength(10);
		expect(
			select.querySelectorAll('optgroup[label="Agent status"] option'),
		).toHaveLength(6);
		await act(async () => {
			select.value = "state:offline";
			select.dispatchEvent(new Event("change", { bubbles: true }));
		});
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-agent-state"),
		).toBe("offline");
		await act(async () => {
			select.value = "mood:love";
			select.dispatchEvent(new Event("change", { bubbles: true }));
		});
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-agent-state"),
		).toBe("idle");
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-mood"),
		).toBe("love");
		expect(onChange).not.toHaveBeenCalled();
		await click("Save bot");
		expect(onSave).toHaveBeenCalledWith(value);
		await click("Wink", group("Preview expression"));
		expect(select.value).toBe("custom");
		expect(
			container.querySelector('[role="img"]')?.getAttribute("data-mood"),
		).toBeNull();
	});

	it("does not block unrelated edits when saved colors contain an unrecognized selection", async () => {
		const initial = {
			kind: "round",
			colors: { body: { kind: "bogus" } },
		} as unknown as BotAppearance;
		const onChange = await mount(initial);
		await section("Finish");
		await click("Choose Stripes");
		expect(onChange).toHaveBeenLastCalledWith({
			...initial,
			backdrop: "stripes",
		});
	});

	it("uses controlled values and native section controls linked to the active panel", async () => {
		const value: BotAppearance = { kind: "round", frame: "circle" };
		const onChange = vi.fn();
		await act(async () =>
			root.render(
				<BotCustomizer value={value} catalog={catalog} onChange={onChange} />,
			),
		);
		await click("Choose Angular bot");
		expect(onChange).toHaveBeenCalledWith({
			kind: "angular",
			frame: "circle",
			equipment: {},
		});
		expect(button("Choose Round bot").getAttribute("aria-pressed")).toBe(
			"true",
		);
		expect(button("Choose Angular bot").getAttribute("aria-pressed")).toBe(
			"false",
		);
		await section("Outfit");
		const pressed = group("Customization sections").querySelector(
			'[aria-pressed="true"]',
		);
		const panel = container.querySelector("section");
		expect(pressed?.getAttribute("aria-controls")).toBe(panel?.id);
		expect(panel?.getAttribute("aria-labelledby")).toBe(pressed?.id);
		expect(
			[...container.querySelectorAll("button")].every(
				(item) => item.type === "button",
			),
		).toBe(true);
	});
});
