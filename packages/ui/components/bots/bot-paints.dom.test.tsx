// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BotAvatar } from "./bot-avatar";
import { BotPaintDefs } from "./bot-paints";
import { createBotCatalog, defaultBotCatalog } from "./catalog";
import {
	BOT_COLOR_SLOTS,
	type BotAppearance,
	type BotAvatarProps,
} from "./types";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.stubGlobal("matchMedia", () => ({
		matches: true,
		addEventListener: vi.fn(),
		removeEventListener: vi.fn(),
	}));
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	vi.unstubAllGlobals();
});

const allPaints = Object.fromEntries(
	BOT_COLOR_SLOTS.map((slot, index) => [
		slot,
		{
			kind: "swatch",
			colorId: index % 2 ? "pattern-stars" : "holo-aurora",
		},
	]),
) as BotAppearance["colors"];

async function render(props: BotAvatarProps = {}) {
	await act(async () => root.render(<BotAvatar animated={false} {...props} />));
}

function assertLocalReferences(avatar: Element) {
	const ids = new Set(
		Array.from(avatar.querySelectorAll("[id]"), (element) => element.id),
	);
	for (const element of avatar.querySelectorAll(
		"[fill], [stroke], [mask], [clip-path]",
	)) {
		for (const name of ["fill", "stroke", "mask", "clip-path"]) {
			const value = element.getAttribute(name) ?? "";
			if (value.startsWith("url(")) {
				const reference = /^url\(#([\w-]+)\)$/.exec(value);
				expect(reference).not.toBeNull();
				expect(ids.has(reference?.[1] ?? "")).toBe(true);
			}
		}
	}
}

describe("bot SVG paints", () => {
	it("renders complex paints in all seven roles without leaking URLs into CSS colors", async () => {
		await render({
			appearance: {
				kind: "flux",
				colors: allPaints,
				backdrop: "gradient",
				equipment: {
					neck: "hoodie",
					head: "crown",
					hand: "mug",
					eyes: "glasses",
				},
				skills: ["web", "code", "chat", "mail"],
			},
		});
		expect(host.querySelectorAll("[data-paint-slot]")).toHaveLength(7);
		for (const slot of BOT_COLOR_SLOTS) {
			const paint = host.querySelector(`[data-paint-slot="${slot}"]`);
			expect(paint).not.toBeNull();
			expect(
				host.querySelector(
					`[fill="url(#${paint?.id})"], [stroke="url(#${paint?.id})"]`,
				),
			).not.toBeNull();
		}
		for (const element of host.querySelectorAll(
			"stop, [color], .fl-bot-badge",
		)) {
			expect(element.getAttribute("stop-color") ?? "").not.toContain("url(");
			expect(element.getAttribute("color") ?? "").not.toContain("url(");
			expect(element.getAttribute("style") ?? "").not.toMatch(
				/background[^:]*:\s*url/,
			);
		}
		assertLocalReferences(host);
	});

	it("gives each thumbnail its own paint definitions and mask references", async () => {
		await act(async () =>
			root.render(
				<>
					<BotAvatar
						animated={false}
						appearance={{ kind: "flux", colors: allPaints }}
					/>
					<BotAvatar
						animated={false}
						appearance={{ kind: "flux", colors: allPaints }}
					/>
				</>,
			),
		);
		const ids = Array.from(
			host.querySelectorAll("[id]"),
			(element) => element.id,
		);
		expect(new Set(ids).size).toBe(ids.length);
		for (const avatar of host.querySelectorAll(".fl-bot"))
			assertLocalReferences(avatar);
	});

	it("keeps custom paint visible through the original Flow and Slate color blends", async () => {
		for (const kind of ["flux", "lava", "magma", "slate"]) {
			await render({
				appearance: {
					kind,
					colors: {
						body: { kind: "swatch", colorId: "pattern-stars" },
						accent: { kind: "swatch", colorId: "holo-aurora" },
						cheek: { kind: "swatch", colorId: "pattern-dots" },
					},
				},
			});
			const body = host.querySelector('[data-paint-slot="body"]');
			expect(
				host
					.querySelector('[data-part="body"] [data-blend-base="body"]')
					?.getAttribute("fill"),
			).toBe(`url(#${body?.id})`);
			const from = kind === "lava" ? "cheek" : "accent";
			const overlay = host.querySelector(
				`[data-part="body"] [data-blend-overlay="${from}"]`,
			);
			expect(overlay?.getAttribute("fill")).toBe(
				`url(#${host.querySelector(`[data-paint-slot="${from}"]`)?.id})`,
			);
			expect(overlay?.getAttribute("mask")).toMatch(/^url\(#/);
			for (const stop of host.querySelectorAll("stop"))
				expect(stop.getAttribute("stop-color")).not.toContain("url(");
			assertLocalReferences(host);
		}
	});

	it("lets palette effects override paints and preserves the dark background", async () => {
		for (const effect of ["lcd", "blueprint", "thermal"] as const) {
			await render({ appearance: { colors: allPaints, effect } });
			expect(host.querySelectorAll("[data-paint-slot]")).toHaveLength(0);
			expect(
				host.querySelector('[data-part="bg"] rect')?.getAttribute("fill"),
			).toBe(
				effect === "lcd"
					? "#d6dfad"
					: effect === "thermal"
						? "#17122d"
						: "#123e6b",
			);
		}
		await render({ appearance: { colors: allPaints, effect: "neon" } });
		expect(host.querySelector('[data-paint-slot="background"]')).toBeNull();
		expect(host.querySelector('[data-paint-slot="body"]')).not.toBeNull();
		expect(
			host.querySelector('[data-part="bg"] rect')?.getAttribute("fill"),
		).toBe("#100e1d");
		await render({ appearance: { colors: allPaints, dark: true } });
		expect(host.querySelector('[data-paint-slot="background"]')).toBeNull();
		expect(host.querySelector('[data-paint-slot="body"]')).not.toBeNull();
		expect(
			host.querySelector('[data-part="bg"] rect')?.getAttribute("fill"),
		).toBe("#232335");
	});

	it("uses user-space paints on closed eye strokes", async () => {
		await render({
			pose: "sleep",
			appearance: {
				kind: "pebble",
				colors: { ink: { kind: "swatch", colorId: "pattern-stripes" } },
			},
		});
		const paint = host.querySelector('[data-paint-slot="ink"]');
		expect(paint?.getAttribute("patternUnits")).toBe("userSpaceOnUse");
		expect(
			host
				.querySelector('[data-feature="closed-eyes"]')
				?.getAttribute("stroke"),
		).toBe(`url(#${paint?.id})`);
	});

	it("retains representative solids and complex slot metadata for shape extensions", async () => {
		const catalog = createBotCatalog({
			shapes: [
				{
					...defaultBotCatalog.shapes.mochi,
					id: "paint-probe",
					renderDetails: ({ solidPalette, paintedSlots }) => (
						<g
							data-solid={solidPalette?.body}
							data-slots={paintedSlots?.join(",")}
						/>
					),
				},
			],
		});
		await render({
			catalog,
			appearance: {
				kind: "paint-probe",
				colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
			},
		});
		expect(host.querySelector("[data-solid]")?.getAttribute("data-solid")).toBe(
			"#68DDD2",
		);
		expect(host.querySelector("[data-slots]")?.getAttribute("data-slots")).toBe(
			"body",
		);
	});

	it("uses CSS animation hooks only for animated gradients", async () => {
		await render({
			appearance: {
				colors: { body: { kind: "swatch", colorId: "holo-aurora" } },
			},
		});
		expect(host.querySelectorAll(".fl-bot-paint-stop")).toHaveLength(3);
		expect(host.querySelectorAll("animate, animateTransform")).toHaveLength(0);
		await act(async () =>
			root.render(
				<svg aria-hidden="true">
					<defs>
						<BotPaintDefs
							idPrefix="static-swatch"
							paints={{
								body: {
									kind: "gradient",
									colors: ["#000000", "#FFFFFF"],
									animated: false,
								},
							}}
						/>
					</defs>
				</svg>,
			),
		);
		expect(host.querySelectorAll(".fl-bot-paint-stop")).toHaveLength(0);
		expect(host.querySelectorAll("stop")).toHaveLength(2);
	});
});
