// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BOT_OPTIONS, resolveBotAppearance } from "./appearance";
import { BotAvatar } from "./bot-avatar";
import { createBotCatalog, defaultBotCatalog } from "./catalog";
import { BOT_EXPRESSIONS, type BotAvatarProps } from "./types";

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.stubGlobal("matchMedia", () => ({
		matches: true,
		addEventListener: vi.fn(),
		removeEventListener: vi.fn(),
	}));
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

async function render(props: BotAvatarProps = {}) {
	await act(async () => root.render(<BotAvatar {...props} />));
	return container.querySelector(".fl-bot") as HTMLDivElement;
}

function part(name: string) {
	const result = container.querySelector(`[data-part="${name}"]`);
	if (!result) throw new Error(`Missing rig part: ${name}`);
	return result;
}

function skills() {
	return Array.from(container.querySelectorAll("[data-skill]"), (badge) =>
		badge.getAttribute("data-skill"),
	);
}

describe("bot avatar rig", () => {
	it("keeps thermal features visible without darkening thought bubbles", async () => {
		await render({
			appearance: { kind: "flux", effect: "thermal", emote: "heart" },
			expression: "happy",
		});
		expect(
			part("eyes")
				.querySelector('[data-part="pupils"]')
				?.getAttribute("stroke"),
		).toBe("#5e126d");
		expect(
			part("emote").querySelector("[data-emote-bubble]")?.getAttribute("fill"),
		).toBe("#ffe95b");
	});
	it.each(BOT_OPTIONS.effect)(
		"keeps expressions, gear and bubbles in the shared %s rig",
		async (effect) => {
			const appearance = resolveBotAppearance(
				{
					kind: "mochi",
					effect,
					equipment: { neck: "sweater", head: "crown" },
					emote: "heart",
				},
				defaultBotCatalog,
			);
			expect(JSON.parse(JSON.stringify(appearance)).effect).toBe(effect);
			const avatar = await render({
				appearance,
				expression: "wink",
				pose: "track",
				gx: 0.5,
				gy: -0.5,
			});
			expect(avatar.getAttribute("data-effect")).toBe(effect);
			expect(avatar.getAttribute("data-expression")).toBe("wink");
			expect(avatar.style.getPropertyValue("--bot-gaze-x")).toBe("1.75px");
			const rig = container.querySelector(".fl-bot-effect-rig");
			expect(rig?.contains(part("head"))).toBe(true);
			expect(rig?.contains(part("neck"))).toBe(true);
			expect(rig?.contains(part("emote"))).toBe(true);
			expect(
				part("emote").querySelector('[data-emote-icon="heart"]'),
			).not.toBeNull();
			for (const element of container.querySelectorAll("svg *")) {
				for (const attribute of element.attributes) {
					for (const match of attribute.value.matchAll(/url\(#([^)]*)\)/g)) {
						expect(
							container.querySelector(`[id="${match[1]}"]`),
						).not.toBeNull();
					}
				}
			}
		},
	);

	it("gives agent status priority and restores the saved appearance afterward", async () => {
		const appearance = {
			kind: "mochi",
			emote: "note" as const,
			effect: "lcd" as const,
			equipment: { head: "crown", hand: "laptop" },
		};
		const snapshot = JSON.stringify(appearance);
		let avatar = await render({
			appearance,
			mood: "love",
			expression: "angry",
			agentState: "working",
		});
		expect(avatar.getAttribute("data-expression")).toBe("focused");
		expect(avatar.getAttribute("data-pose")).toBe("center");
		expect(avatar.getAttribute("aria-label")).toContain("Working");
		expect(part("emote").querySelectorAll("[data-loading-dot]")).toHaveLength(
			3,
		);
		expect(part("emote").getAttribute("data-visible")).toBe("true");
		const outfit = part("head").innerHTML + part("hand").innerHTML;
		avatar = await render({ appearance, mood: "love", agentState: "offline" });
		expect(avatar.getAttribute("data-effect")).toBe("offline");
		expect(avatar.getAttribute("data-pose")).toBe("sleep");
		expect(
			container.querySelector('[data-feature="closed-eyes"]'),
		).not.toBeNull();
		avatar = await render({ appearance, mood: "love", agentState: "idle" });
		expect(avatar.getAttribute("data-effect")).toBe("lcd");
		expect(avatar.getAttribute("data-expression")).toBe("love");
		expect(
			part("emote").querySelector('[data-emote-icon="heart"]'),
		).not.toBeNull();
		expect(part("head").innerHTML + part("hand").innerHTML).toBe(outfit);
		await render({ appearance });
		expect(
			part("emote").querySelector('[data-emote-icon="note"]'),
		).not.toBeNull();
		expect(JSON.stringify(appearance)).toBe(snapshot);
	});

	it("pops mood bubbles during idle and keeps active status bubbles visible", async () => {
		vi.useFakeTimers();
		vi.spyOn(Math, "random").mockReturnValue(0);
		vi.spyOn(document, "hidden", "get").mockReturnValue(false);
		vi.stubGlobal("matchMedia", () => ({
			matches: false,
			addEventListener: vi.fn(),
			removeEventListener: vi.fn(),
		}));
		await render({ mood: "wink" });
		expect(part("emote").getAttribute("data-visible")).toBe("false");
		await act(async () => vi.advanceTimersByTime(4800));
		expect(part("emote").getAttribute("data-visible")).toBe("true");
		await act(async () => vi.advanceTimersByTime(1800));
		expect(part("emote").getAttribute("data-visible")).toBe("false");
		await render({ agentState: "working" });
		expect(part("emote").getAttribute("data-visible")).toBe("true");
		await act(async () => vi.advanceTimersByTime(9000));
		expect(part("emote").getAttribute("data-visible")).toBe("true");
	});

	it("keeps gaze and blinking active as the facial expression changes", async () => {
		vi.useFakeTimers();
		vi.spyOn(Math, "random").mockReturnValue(0);
		vi.spyOn(document, "hidden", "get").mockReturnValue(false);
		vi.stubGlobal("matchMedia", () => ({
			matches: false,
			addEventListener: vi.fn(),
			removeEventListener: vi.fn(),
		}));
		await render({ pose: "track", gx: 0.5, gy: -0.5, expression: "happy" });
		expect(container.querySelector(".fl-bot-happy-eye")).not.toBeNull();
		expect(
			container
				.querySelector<HTMLElement>(".fl-bot")
				?.style.getPropertyValue("--bot-gaze-x"),
		).toBe("1.75px");
		await act(async () => vi.advanceTimersByTime(3500));
		await render({ pose: "track", gx: 0.5, gy: -0.5, expression: "concerned" });
		await act(async () => vi.advanceTimersByTime(100));
		expect(part("eyes").getAttribute("data-blinking")).toBe("true");
		expect(container.querySelector('[data-feature="brows"]')).not.toBeNull();
		expect(container.querySelector(".fl-bot-happy-eye")).toBeNull();
		await act(async () => vi.advanceTimersByTime(150));
		expect(part("eyes").getAttribute("data-blinking")).toBe("false");
	});

	it("renders distinct expressions across the crew and preserves mouthless shapes", async () => {
		for (const kind of ["mochi", "flux", "hami"]) {
			const faces = new Set<string>();
			for (const expression of BOT_EXPRESSIONS) {
				const avatar = await render({
					appearance: { kind },
					expression,
					animated: false,
				});
				expect(avatar.getAttribute("data-expression")).toBe(expression);
				faces.add(
					part("eyes").innerHTML.replace(/bot-[^"#) ]+/g, "bot-id") +
						part("face").innerHTML,
				);
			}
			expect(faces.size).toBe(BOT_EXPRESSIONS.length);
		}
		const mouthless = Object.values(defaultBotCatalog.shapes).find(
			(shape) => shape.mouthStyle === "none",
		);
		if (!mouthless) throw new Error("Expected a mouthless shape in the crew");
		await render({
			appearance: { kind: mouthless.id },
			expression: "concerned",
		});
		expect(container.querySelector('[data-feature="mouth"]')).toBeNull();
		expect(
			container.querySelectorAll('[data-feature="brows"] path'),
		).toHaveLength(2);
	});

	it("preserves legacy expression poses and lets an explicit expression override them", async () => {
		await render({ pose: "happy" });
		expect(container.querySelector(".fl-bot-happy-eye")).not.toBeNull();
		await render({ pose: "oops" });
		expect(
			container.querySelector('[data-feature="mouth"] path'),
		).not.toBeNull();
		expect(
			container.querySelector(".fl-bot")?.getAttribute("data-expression"),
		).toBe("oops");
		await render({ pose: "happy", expression: "neutral" });
		expect(container.querySelector(".fl-bot-happy-eye")).toBeNull();
	});

	it("keeps equipment on its intended moving parent and paints the reference layer order", async () => {
		await render({
			appearance: {
				equipment: {
					neck: "scarf",
					eyes: "glasses",
					hand: "laptop",
					head: "crown",
				},
				emote: "heart",
				halo: "flow",
				skills: ["web"],
			},
		});
		const ordered = [
			"bg",
			"hop",
			"body",
			"neck",
			"eyes",
			"pupils",
			"face",
			"hand",
			"head",
			"emote",
		];
		for (let i = 1; i < ordered.length; i++) {
			expect(
				part(ordered[i - 1]).compareDocumentPosition(part(ordered[i])) &
					Node.DOCUMENT_POSITION_FOLLOWING,
			).not.toBe(0);
		}
		for (const name of ["body", "neck", "eyes", "face", "hand", "head"]) {
			expect(part("hop").contains(part(name))).toBe(true);
		}
		expect(part("eyes").contains(part("pupils"))).toBe(true);
		expect(
			part("face").querySelector('[data-slot="eyes"]')?.children.length,
		).toBeGreaterThan(0);
		expect(part("hop").contains(part("emote"))).toBe(false);
		const bodyClip = part("neck")
			.getAttribute("clip-path")
			?.match(/^url\(#(.+)\)$/)?.[1];
		expect(
			document
				.getElementById(bodyClip ?? "")
				?.querySelector("path")
				?.getAttribute("d"),
		).toBe(defaultBotCatalog.shapes.mochi.silhouette);
		const svg = container.querySelector("svg");
		expect(svg?.getAttribute("viewBox")).toBe("0 0 100 100");
		expect(svg?.contains(container.querySelector(".fl-bot-halo"))).toBe(false);
		expect(svg?.contains(container.querySelector(".fl-bot-badges"))).toBe(
			false,
		);
	});

	it("gives every instance independent definitions and clip references", async () => {
		await act(async () =>
			root.render(
				<>
					<BotAvatar appearance={{ backdrop: "dots", effect: "raster" }} />
					<BotAvatar appearance={{ backdrop: "dots", effect: "raster" }} />
				</>,
			),
		);
		const ids = Array.from(
			container.querySelectorAll("[id]"),
			(node) => node.id,
		);
		expect(new Set(ids).size).toBe(ids.length);
		for (const avatar of container.querySelectorAll(".fl-bot")) {
			for (const node of avatar.querySelectorAll("[clip-path]")) {
				const id = node.getAttribute("clip-path")?.match(/^url\(#(.+)\)$/)?.[1];
				expect(id).toBeDefined();
				expect(avatar.contains(document.getElementById(id ?? ""))).toBe(true);
			}
		}
	});

	it("omits unavailable, wrong-slot and incompatible equipment while preserving valid items", async () => {
		const catalog = createBotCatalog({
			equipment: [
				{
					id: "sprout-crown",
					label: "Sprout crown",
					slot: "head",
					kinds: ["sprout"],
					render: () => <path data-test-item="sprout-crown" d="M30 30H70" />,
				},
			],
		});
		await render({
			catalog,
			appearance: {
				kind: "mochi",
				equipment: {
					neck: "glasses",
					eyes: "unavailable",
					head: "sprout-crown",
					hand: "laptop",
				},
			},
		});
		expect(part("neck").children).toHaveLength(0);
		expect(
			part("face").querySelector('[data-slot="eyes"]')?.children,
		).toHaveLength(0);
		expect(part("head").children).toHaveLength(0);
		expect(part("hand").children.length).toBeGreaterThan(0);
		await render({
			catalog,
			appearance: { kind: "sprout", equipment: { head: "sprout-crown" } },
		});
		expect(
			part("head").querySelector('[data-test-item="sprout-crown"]'),
		).not.toBeNull();
	});

	it.each([
		[64, ["web", "code", "chat"], null],
		[63, ["web", "overflow"], "+2"],
		[40, ["web", "overflow"], "+2"],
		[39, ["overflow"], "+3"],
	] as const)(
		"limits the skill arc at %i px and accounts for every hidden skill",
		async (size, expected, counter) => {
			const avatar = await render({
				size,
				appearance: { skills: ["web", "code", "chat"] },
			});
			expect(skills()).toEqual(expected);
			expect(
				container.querySelector('[data-skill="overflow"]')?.textContent ?? null,
			).toBe(counter);
			expect(avatar.getAttribute("aria-label")).toBe(
				"Mochi bot; skills: Web, Code, Chat",
			);
		},
	);

	it("converts a hand item below 56 px, deduplicates its skill and drops the mug", async () => {
		await render({ size: 56, appearance: { equipment: { hand: "laptop" } } });
		expect(part("hand").children.length).toBeGreaterThan(0);
		expect(skills()).toEqual([]);
		await render({ size: 55, appearance: { equipment: { hand: "laptop" } } });
		expect(part("hand").children).toHaveLength(0);
		expect(skills()).toEqual(["device"]);
		await render({
			size: 55,
			appearance: {
				equipment: { hand: "laptop" },
				skills: ["device", "device"],
			},
		});
		expect(skills()).toEqual(["device"]);
		await render({ size: 55, appearance: { equipment: { hand: "mug" } } });
		expect(part("hand").children).toHaveLength(0);
		expect(skills()).toEqual([]);
	});

	it.each(["lcd", "blueprint"] as const)(
		"lets %s recolor equipment and override a body variant",
		async (effect) => {
			await render({
				appearance: { variant: "gold", equipment: { neck: "sweater" } },
			});
			const originalClothing = part("neck")
				.querySelector("path")
				?.getAttribute("fill");
			expect(
				part("body").querySelector(".fl-bot-skin")?.getAttribute("filter"),
			).toContain("-variant)");
			expect(part("neck").hasAttribute("filter")).toBe(false);
			await render({
				appearance: { variant: "gold", effect, equipment: { neck: "sweater" } },
			});
			expect(
				part("body")
					.querySelector(".fl-bot-skin")
					?.getAttribute("data-variant"),
			).toBe("original");
			expect(
				part("body").querySelector(".fl-bot-skin")?.hasAttribute("filter"),
			).toBe(false);
			const bodyColor = part("body")
				.querySelector("path")
				?.getAttribute("fill");
			const clothingColor = part("neck")
				.querySelector("path")
				?.getAttribute("fill");
			expect(clothingColor).not.toBe(originalClothing);
			await render({
				appearance: { effect, variant: "holo", equipment: { neck: "sweater" } },
			});
			expect(
				part("body")
					.querySelector(".fl-bot-skin")
					?.getAttribute("data-variant"),
			).toBe("original");
			expect(part("body").querySelector("path")?.getAttribute("fill")).toBe(
				bodyColor,
			);
			expect(part("neck").querySelector("path")?.getAttribute("fill")).toBe(
				clothingColor,
			);
		},
	);

	it("keeps emotes visible with reduced motion and supports decorative rendering", async () => {
		const avatar = await render({
			appearance: { emote: "heart", halo: "flow" },
		});
		expect(avatar.getAttribute("data-motion")).toBe("off");
		expect(part("emote").getAttribute("data-visible")).toBe("true");
		expect(
			part("emote").querySelector('[data-emote-icon="heart"]'),
		).not.toBeNull();
		await render({
			decorative: true,
			appearance: { emote: "heart" },
			animated: false,
		});
		expect(avatar.getAttribute("aria-hidden")).toBe("true");
		expect(avatar.hasAttribute("role")).toBe(false);
		expect(avatar.hasAttribute("aria-label")).toBe(false);
		expect(part("emote").getAttribute("data-visible")).toBe("true");
	});

	it("renders extension art with the shape's anchors, palette and unique definition prefix", async () => {
		const shape = defaultBotCatalog.shapes.mochi;
		const catalog = createBotCatalog({
			defaultKind: "custom",
			shapes: [
				{
					...shape,
					id: "custom",
					label: "Custom",
					silhouette: "M10 20H90V90H10Z",
					anchors: {
						...shape.anchors,
						eyes: { left: [30, 45], right: [70, 45], radius: 9 },
						head: { x: 50, y: 20, width: 80 },
					},
					palette: { ...shape.palette, accent: "#123456" },
					renderDetails: ({ idPrefix }) => (
						<g data-test-detail={idPrefix}>
							<circle cx="50" cy="85" r="2" />
						</g>
					),
				},
			],
			equipment: [
				{
					id: "custom-hat",
					label: "Custom hat",
					slot: "head",
					render: ({ anchors, palette, idPrefix }) => (
						<circle
							data-test-hat={idPrefix}
							cx={anchors.head.x}
							cy={anchors.head.y}
							r={anchors.head.width / 4}
							fill={palette.accent}
						/>
					),
				},
			],
		});
		const avatar = await render({
			catalog,
			appearance: { equipment: { head: "custom-hat" } },
		});
		expect(avatar.getAttribute("data-kind")).toBe("custom");
		expect(part("body").querySelector("path")?.getAttribute("d")).toBe(
			"M10 20H90V90H10Z",
		);
		expect(part("eyes").querySelector(".sclera")?.getAttribute("cx")).toBe(
			"30",
		);
		expect(part("eyes").querySelector(".sclera")?.getAttribute("rx")).toBe("9");
		const hat = part("head").querySelector("circle");
		expect(hat?.getAttribute("cy")).toBe("20");
		expect(hat?.getAttribute("r")).toBe("20");
		expect(hat?.getAttribute("fill")).toBe("#123456");
		expect(hat?.getAttribute("data-test-hat")).toBe(
			part("body")
				.querySelector("[data-test-detail]")
				?.getAttribute("data-test-detail"),
		);
	});

	it("falls back for removed shape and skill IDs without exposing prototype names", async () => {
		const avatar = await render({
			appearance: {
				kind: "constructor",
				skills: ["toString", "unknown", "web"],
			},
		});
		expect(avatar.getAttribute("data-kind")).toBe(
			defaultBotCatalog.defaultKind,
		);
		expect(skills()).toEqual(["web"]);
	});
});
