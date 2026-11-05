// @vitest-environment happy-dom

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { BotAvatar } from "./bot-avatar";
import { defaultBotCatalog } from "./catalog";

const CREW = {
	flow: ["flux", "lava", "magma", "bubble"],
	basics: ["pip", "mochi", "hexa", "pebble", "nimbus"],
	nature: ["sprout", "drip", "ember", "porci", "bloom"],
	pets: ["miso", "bun", "kuma", "fynn", "hami"],
	zoo: ["kero", "axi", "pino", "piyo", "bambu"],
	tech: ["nodi", "bit", "chip", "cog", "floppy"],
	snacks: ["momo", "nori", "toasty", "cupi", "puri"],
	cosmos: ["orbi", "sol", "stella", "roki", "astro"],
	pro: ["atlas", "prism", "slate", "facet", "otto"],
	office: ["clip", "jot", "memo", "rubi", "ruler", "binder", "klick"],
	workshop: ["wrench", "mallet", "bolt", "brush", "spool", "measure"],
	ocean: ["inky", "shelly", "bubbles", "cora", "jelly", "pearl"],
	fantasy: ["wisp", "draco", "rune", "glimmer", "wish", "keep"],
	garden: ["prickles", "bonsai", "fern", "acorn", "clover", "berry"],
	music: ["tempo", "vinyl", "boombox", "strum", "clef", "chime"],
	adventure: ["scout", "summit", "camp", "beacon", "skipper", "globe"],
	arcade: ["pixel", "joystick", "dicey", "puzzlo", "marble", "domino"],
	pirates: ["helm", "coffer", "spyglass", "cask", "anchor", "squawk"],
	arcane: ["brim", "phial", "broom", "sands", "brew", "emberwick"],
	celebrations: ["noelle", "jacklet", "paska", "confetti", "hong", "diya"],
	service: ["radio", "hydrant", "rotor", "rucksack", "pylon", "buoy"],
	workplace: ["brief", "dial", "calc", "type", "badge", "agenda"],
	healthcare: ["scope", "aid", "capsule", "molar", "pipette", "pulse"],
};

function find(container: Element, selector: string): Element {
	const result = container.querySelector(selector);
	if (!result) throw new Error(`Missing ${selector}`);
	return result;
}

function renderCrew(equipped = false) {
	const container = document.createElement("div");
	container.innerHTML = renderToStaticMarkup(
		<div>
			{Object.keys(defaultBotCatalog.shapes).map((kind) => (
				<BotAvatar
					key={kind}
					appearance={{
						kind,
						equipment: equipped
							? {
									head: "crown",
									eyes: "glasses",
									neck: "hoodie",
									hand: "laptop",
								}
							: undefined,
					}}
					animated={false}
				/>
			))}
		</div>,
	);
	return container;
}

describe("bot crew", () => {
	it("gives sleep closed eyelids and a resting mouth, even with another expression selected", () => {
		for (const kind of ["mochi", "bambu", "astro", "drip"]) {
			const container = document.createElement("div");
			container.innerHTML = renderToStaticMarkup(
				<BotAvatar
					appearance={{ kind }}
					pose="sleep"
					expression="surprised"
					animated={false}
				/>,
			);
			expect(
				container.querySelectorAll('[data-feature="closed-eyes"] path'),
			).toHaveLength(2);
			expect(container.querySelector('[data-part="eyes"] .sclera')).toBeNull();
			expect(
				container.querySelector('[data-part="eyes"] [data-part="pupils"]'),
			).toBeNull();
			expect(
				container
					.querySelector('[data-part="eyes"]')
					?.hasAttribute("transform"),
			).toBe(false);
			expect(container.querySelector('[data-feature="brows"]')).toBeNull();
			expect(
				container.querySelector('[data-part="emote"]')?.textContent,
			).toContain("zZ");
			if (kind === "drip")
				expect(container.querySelector('[data-feature="mouth"]')).toBeNull();
			else
				expect(
					container
						.querySelector('[data-feature="mouth"] path')
						?.getAttribute("d"),
				).toBe("M-2 0q2 1.5 4 0");
		}
	});

	it("keeps an explicit status emote when the bot sleeps", () => {
		const container = document.createElement("div");
		container.innerHTML = renderToStaticMarkup(
			<BotAvatar
				appearance={{ kind: "mochi", emote: "heart" }}
				pose="sleep"
				animated={false}
			/>,
		);
		expect(
			container.querySelector('[data-part="emote"] [data-emote-icon="heart"]'),
		).not.toBeNull();
	});

	it("contains every named character in its expected series", () => {
		for (const [series, ids] of Object.entries(CREW)) {
			expect(
				Object.values(defaultBotCatalog.shapes)
					.filter((shape) => shape.series === series)
					.map((shape) => shape.id),
			).toEqual(ids);
		}
	});

	it("renders the entire wardrobe on every character with isolated SVG definitions", () => {
		const container = renderCrew(true);
		expect(container.querySelectorAll(".fl-bot")).toHaveLength(
			Object.values(CREW).flat().length,
		);
		const ids = Array.from(
			container.querySelectorAll("[id]"),
			(element) => element.id,
		);
		expect(new Set(ids).size).toBe(ids.length);
		for (const bot of container.querySelectorAll(".fl-bot")) {
			for (const slot of ["head", "neck", "hand"])
				expect(
					bot.querySelector(`[data-part="${slot}"]`)?.childElementCount,
				).toBeGreaterThan(0);
			expect(
				bot.querySelector('[data-slot="eyes"]')?.childElementCount,
			).toBeGreaterThan(0);
			expect(bot.innerHTML).not.toMatch(/NaN|undefined|Infinity/);
		}
	});

	it("keeps character-specific eyes and mouths instead of giving every shape one face", () => {
		const container = renderCrew();
		const bot = (id: string) => find(container, `[data-kind="${id}"]`);
		expect(bot("drip").querySelector('[data-feature="mouth"]')).toBeNull();
		expect(bot("mochi").querySelectorAll(".sclera")).toHaveLength(2);
		expect(bot("pip").querySelectorAll(".sclera")).toHaveLength(0);
		expect(bot("hexa").querySelectorAll(".lid")).toHaveLength(2);
		expect(
			bot("flux")
				.querySelector('[data-part="pupils"] ellipse')
				?.getAttribute("fill"),
		).toBe(defaultBotCatalog.shapes.flux.palette.sclera);
		for (const id of ["pino", "piyo", "otto"])
			expect(
				bot(id)
					.querySelector('[data-feature="mouth"] path')
					?.getAttribute("fill"),
			).toBe(defaultBotCatalog.shapes[id].palette.accent);
	});

	it("keeps happy eyes clear of visor and animal markings", () => {
		for (const kind of ["astro", "bambu", "pino", "toasty", "kero"]) {
			const container = document.createElement("div");
			container.innerHTML = renderToStaticMarkup(
				<BotAvatar appearance={{ kind }} pose="happy" animated={false} />,
			);
			const pupils = find(container, '[data-part="pupils"]');
			expect(pupils.getAttribute("fill")).toBe("none");
			expect(pupils.querySelectorAll("path")).toHaveLength(2);
			expect(
				container.querySelector('[data-part="eyes"] path[fill]'),
			).toBeNull();
		}
	});

	it("keeps Astro's facial features distinct from its visor in palette effects", () => {
		for (const effect of ["none", "lcd", "blueprint"] as const) {
			const container = document.createElement("div");
			container.innerHTML = renderToStaticMarkup(
				<BotAvatar appearance={{ kind: "astro", effect }} animated={false} />,
			);
			const visor = find(container, '[data-part="body"] ellipse').getAttribute(
				"fill",
			);
			const eye = find(container, '[data-part="pupils"] ellipse').getAttribute(
				"fill",
			);
			const mouth = find(container, '[data-feature="mouth"] path').getAttribute(
				"stroke",
			);
			expect(eye).toBe(mouth);
			expect(eye).not.toBe(visor);
		}
	});

	it("keeps Pebble's glasses and Slate's clothes in the accessory slots", () => {
		const plain = renderCrew();
		const equipped = renderCrew(true);
		expect(defaultBotCatalog.shapes.pebble.renderFaceDetails).toBeUndefined();
		expect(
			find(plain, '[data-kind="pebble"] [data-slot="eyes"]').childElementCount,
		).toBe(0);
		expect(
			find(
				equipped,
				'[data-kind="pebble"] [data-slot="eyes"]',
			).querySelectorAll("ellipse"),
		).toHaveLength(2);
		expect(
			find(plain, '[data-kind="slate"] [data-part="neck"]').childElementCount,
		).toBe(0);
		expect(
			find(equipped, '[data-kind="slate"] [data-part="neck"]')
				.childElementCount,
		).toBeGreaterThan(0);
		const bodyPaths = (container: Element) =>
			[
				...container.querySelectorAll(
					'[data-kind="slate"] [data-part="body"] path',
				),
			].map((path) => path.getAttribute("d"));
		expect(bodyPaths(plain)).toEqual(bodyPaths(equipped));
	});

	it("layers a complete Orbi ring around the body with instance-scoped foreground clips", () => {
		const container = renderCrew();
		const skin = find(container, '[data-kind="orbi"] .fl-bot-skin');
		expect(skin.firstElementChild?.getAttribute("data-motion-part")).toBe(
			"orbit-back",
		);
		expect(skin.children[1].getAttribute("d")).toBe(
			defaultBotCatalog.shapes.orbi.silhouette,
		);
		expect(
			skin.querySelector('[data-motion-part="orbit-back"] ellipse'),
		).not.toBeNull();
		expect(
			skin.querySelector('[data-motion-part="orbit-front"] path'),
		).not.toBeNull();
		expect(skin.querySelectorAll(".fl-bot-orbit-moon")).toHaveLength(2);
	});
});
