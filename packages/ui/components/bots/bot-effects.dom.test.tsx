// @vitest-environment happy-dom

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { BOT_OPTIONS } from "./appearance";
import {
	BotEffectDefs,
	BotEffectOverlay,
	getBotEffectFilter,
} from "./bot-effects";
import { defaultBotCatalog } from "./catalog";
import type { BotEffect } from "./types";

function renderEffects(effects: readonly BotEffect[]) {
	const host = document.createElement("div");
	host.innerHTML = renderToStaticMarkup(
		<svg aria-hidden="true">
			{effects.map((effect, index) => {
				const props = {
					effect,
					idPrefix: `look-${index}`,
					palette: defaultBotCatalog.shapes.mochi.palette,
				};
				return (
					<g key={props.idPrefix} data-look={effect}>
						<defs>
							<BotEffectDefs {...props} />
						</defs>
						<g filter={getBotEffectFilter(effect, props.idPrefix)}>
							<path d="M20 20h60v60H20z" />
						</g>
						<BotEffectOverlay {...props} />
					</g>
				);
			})}
		</svg>,
	);
	return host;
}

describe("bot look artwork", () => {
	it("keeps definitions instance-local when many copies of every effect share a page", () => {
		const host = renderEffects([...BOT_OPTIONS.effect, ...BOT_OPTIONS.effect]);
		const ids = [...host.querySelectorAll("[id]")].map((element) => element.id);
		expect(new Set(ids).size).toBe(ids.length);
		for (const element of host.querySelectorAll("*")) {
			for (const attribute of element.attributes) {
				for (const match of attribute.value.matchAll(/url\(#([^)]*)\)/g)) {
					expect(ids).toContain(match[1]);
				}
			}
		}
		expect(host.innerHTML).not.toMatch(/NaN|undefined|Infinity/);
		expect(
			host.querySelector("animate, animateTransform, animateMotion"),
		).toBeNull();
	});

	it("leaves original, pixel and offline rendering to their own paths", () => {
		const host = renderEffects(["none", "pixel", "offline"]);
		expect(host.querySelector("filter, [data-effect-overlay]")).toBeNull();
		for (const effect of ["none", "pixel", "offline"] as const)
			expect(getBotEffectFilter(effect, "bot")).toBeUndefined();
	});

	it("maps fixed-color equipment through the same LCD and thermal color transforms", () => {
		const host = renderEffects(["lcd", "thermal", "blueprint"]);
		for (const filter of host.querySelectorAll("filter")) {
			expect(
				filter.querySelector('feColorMatrix[type="saturate"]'),
			).not.toBeNull();
			expect(
				filter.querySelectorAll(
					"feComponentTransfer feFuncR, feComponentTransfer feFuncG, feComponentTransfer feFuncB",
				),
			).toHaveLength(3);
		}
		for (const channel of host.querySelectorAll(
			'[data-effect-filter="lcd"] feComponentTransfer > *',
		)) {
			expect(channel.getAttribute("type")).toBe("discrete");
			expect(channel.getAttribute("tableValues")?.split(" ")).toHaveLength(4);
		}
	});

	it("extends filter bounds for outlines and glow while keeping texture noninteractive", () => {
		const host = renderEffects([
			"sticker",
			"neon",
			"comic",
			"crt",
			"raster",
			"watercolor",
		]);
		for (const filter of host.querySelectorAll("[data-effect-filter]")) {
			expect(Number(filter.getAttribute("x"))).toBeLessThan(0);
			expect(Number(filter.getAttribute("width"))).toBeGreaterThan(100);
		}
		for (const overlay of host.querySelectorAll("[data-effect-overlay]")) {
			expect(overlay.getAttribute("pointer-events")).toBe("none");
		}
	});
});
