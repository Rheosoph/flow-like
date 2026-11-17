import { describe, expect, it } from "bun:test";
import { gooBounds } from "./goo-renderer";
import {
	type BotPlace,
	DenFrame,
	DenScene,
	type DenShape,
	type SceneInput,
	stretched,
} from "./scene";
import { Spring } from "./spring";

const SCREEN = 1600;

const topDen: DenShape = {
	edge: "top",
	rect: { x: 754, y: 0, width: 92, height: 30 },
	radius: 15,
	slots: [
		{ botId: "a", x: 784, y: 15.5 },
		{ botId: "b", x: 816, y: 15.5 },
	],
	wrapsNotch: false,
};

/** A den on a board's right edge at x = 1200, two bots stacked along it. */
const sideDen: DenShape = {
	edge: "right",
	rect: { x: 1170, y: 500, width: 30, height: 92 },
	radius: 15,
	slots: [
		{ botId: "a", x: 1185, y: 530 },
		{ botId: "b", x: 1185, y: 562 },
	],
	wrapsNotch: false,
};

function input(
	places: Record<string, BotPlace> = {},
	extra: Partial<SceneInput> = {},
	den: DenShape = topDen,
): SceneInput {
	return {
		den,
		places: new Map(Object.entries(places)),
		pill: null,
		hoverBot: null,
		pull: null,
		screenWidth: SCREEN,
		...extra,
	};
}

function settle(scene: DenScene, seconds = 3): void {
	for (let elapsed = 0; elapsed < seconds; elapsed += 1 / 60)
		scene.step(1 / 60);
}

function perch(visible = true, x = 400): BotPlace {
	return {
		kind: "perch",
		targetId: "w",
		x,
		y: 287,
		edgeY: 300,
		visible,
	};
}

describe("springs", () => {
	it("overshoots a little and comes to rest on the target", () => {
		const spring = new Spring(0);
		spring.set(100);
		let peak = 0;
		for (let frame = 0; frame < 240; frame++) {
			spring.step(1 / 60);
			peak = Math.max(peak, spring.value);
		}
		expect(peak).toBeGreaterThan(100);
		expect(peak).toBeLessThan(130);
		expect(spring.value).toBe(100);
		expect(spring.step(1 / 60)).toBe(false);
	});

	it("rubber-bands a pull", () => {
		expect(stretched(10, 150)).toBeCloseTo(9.7, 0);
		expect(stretched(1000, 150)).toBeLessThan(150);
		expect(stretched(-5, 150)).toBe(0);
	});
});

describe("den frame", () => {
	it("maps along and out onto the top edge", () => {
		const frame = DenFrame.of(topDen);
		expect(frame.point(800, 10)).toEqual({ x: 800, y: 10 });
		expect(frame.split({ x: 4, y: 30 })).toEqual({ along: 4, out: 30 });
	});

	it("maps along and out onto a right edge, out pointing left", () => {
		const frame = DenFrame.of(sideDen);
		expect(frame.at).toBe(1200);
		expect(frame.point(540, 10)).toEqual({ x: 1190, y: 540 });
		expect(frame.split({ x: -30, y: 4 })).toEqual({ along: 4, out: 30 });
		expect(frame.rect(546, 92, 30)).toEqual(sideDen.rect);
	});
});

describe("scene on the top edge", () => {
	it("keeps idle bots in their den slots with only the den drawn", () => {
		const scene = new DenScene();
		scene.setInput(input());
		settle(scene);
		const frame = scene.frame();
		expect(frame.blobs).toHaveLength(1);
		expect(frame.blobs[0].anchored).toBe(true);
		expect(frame.anchor).toEqual({ edge: "top", at: 0 });
		expect(frame.den).toEqual(topDen.rect);
		expect(frame.bots.get("a")).toMatchObject({ x: 784, y: 15.5 });
	});

	it("draws a neck to a bot dragged near the den and lets it pinch off far away", () => {
		const scene = new DenScene();
		scene.setInput(input({ a: { kind: "drag", x: 790, y: 70 } }));
		settle(scene);
		expect(scene.frame().blobs.length).toBeGreaterThan(2);
		scene.setInput(input({ a: { kind: "drag", x: 500, y: 600 } }));
		settle(scene);
		const far = scene.frame();
		expect(far.blobs).toHaveLength(2);
		expect(far.bots.get("a")).toMatchObject({ x: 500, y: 600 });
	});

	it("fades a perched bot whose target is covered and seats a visible one on the edge", () => {
		const scene = new DenScene();
		scene.setInput(input({ a: perch(true) }));
		settle(scene);
		const seated = scene.frame();
		expect(seated.blobs).toHaveLength(3);
		expect(seated.bots.get("a")?.opacity).toBe(1);
		scene.setInput(input({ a: perch(false) }));
		settle(scene);
		const hidden = scene.frame();
		expect(hidden.bots.get("a")?.opacity).toBe(0);
		expect(hidden.blobs).toHaveLength(1);
	});

	it("morphs the bot with the open prompt into a pill kept on screen", () => {
		const scene = new DenScene();
		scene.setInput(
			input({ a: perch(true, 1580) }, { pill: { botId: "a", width: 340 } }),
		);
		settle(scene);
		const pill = scene.frame().bots.get("a")?.pill;
		expect(pill).not.toBeNull();
		expect(pill?.width).toBeCloseTo(340, 0);
		expect((pill?.x ?? 0) + (pill?.width ?? 0)).toBeLessThanOrEqual(SCREEN - 7);
	});

	it("trails a tail behind a fast droplet and none once it rests", () => {
		const scene = new DenScene();
		scene.setInput(input({ a: { kind: "drag", x: 400, y: 300 } }));
		settle(scene);
		scene.setInput(input({ a: { kind: "drag", x: 900, y: 500 } }));
		for (let frame = 0; frame < 4; frame++) scene.step(1 / 60);
		const moving = scene.frame();
		expect(moving.blobs.some((blob) => blob.to !== undefined)).toBe(true);
		expect(Math.abs(moving.bots.get("a")?.tilt ?? 0)).toBeGreaterThan(1);
		settle(scene);
		const resting = scene.frame();
		expect(resting.blobs.some((blob) => blob.to !== undefined)).toBe(false);
		expect(resting.bots.get("a")?.tilt).toBe(0);
	});

	it("squashes a dropped bot as it lands and wobbles back to round", () => {
		const scene = new DenScene();
		scene.setInput(input({ a: { kind: "drag", x: 400, y: 287 } }));
		settle(scene);
		expect(scene.frame().bots.get("a")?.lift).toBe(1);
		scene.setInput(input({ a: perch() }));
		const widths: number[] = [];
		for (let frame = 0; frame < 30; frame++) {
			scene.step(1 / 60);
			widths.push(scene.frame().bots.get("a")?.scaleX ?? 1);
		}
		expect(Math.max(...widths)).toBeGreaterThan(1.05);
		expect(Math.min(...widths)).toBeLessThan(1);
		settle(scene);
		expect(scene.frame().bots.get("a")).toMatchObject({
			scaleX: 1,
			scaleY: 1,
			lift: 0,
		});
	});

	it("recoils the den when a bot leaves it", () => {
		const scene = new DenScene();
		scene.setInput(input());
		settle(scene);
		const rest = scene.frame().den.height;
		scene.setInput(input({ a: { kind: "drag", x: 790, y: 70 } }));
		const heights: number[] = [];
		for (let frame = 0; frame < 12; frame++) {
			scene.step(1 / 60);
			heights.push(scene.frame().den.height);
		}
		expect(Math.min(...heights)).toBeLessThan(rest - 2);
	});

	it("stretches the den while it is pulled and springs back after", () => {
		const scene = new DenScene();
		scene.setInput(input({}, { pull: { x: 0, y: 90 } }));
		settle(scene);
		expect(scene.frame().blobs.length).toBeGreaterThan(1);
		scene.setInput(input());
		settle(scene);
		expect(scene.frame().blobs).toHaveLength(1);
	});
});

describe("scene on a right edge", () => {
	it("anchors the den body to the edge and keeps the bots in their slots", () => {
		const scene = new DenScene();
		scene.setInput(input({}, {}, sideDen));
		settle(scene);
		const frame = scene.frame();
		expect(frame.anchor).toEqual({ edge: "right", at: 1200 });
		expect(frame.den).toEqual(sideDen.rect);
		const body = frame.blobs[0];
		expect(body.anchored).toBe(true);
		expect(body.x + body.hw).toBeCloseTo(1200 + sideDen.radius, 5);
		expect(body.x - body.hw).toBeCloseTo(1170, 5);
		expect(body.hh).toBeCloseTo(46, 5);
		expect(frame.bots.get("b")).toMatchObject({ x: 1185, y: 562 });
	});

	it("pulls a neck out of the den's inner side toward a dragged bot", () => {
		const scene = new DenScene();
		scene.setInput(
			input({ a: { kind: "drag", x: 1120, y: 534 } }, {}, sideDen),
		);
		settle(scene);
		const necks = scene.frame().blobs.filter((blob) => blob.to);
		expect(necks.length).toBeGreaterThan(0);
		expect(necks[0].x).toBeCloseTo(1176, 5);
		expect(necks[0].y).toBeCloseTo(530, 5);
	});

	it("drops the pulled tip into the board, away from the edge", () => {
		const scene = new DenScene();
		scene.setInput(input({}, { pull: { x: -90, y: 0 } }, sideDen));
		settle(scene);
		const blobs = scene.frame().blobs;
		const tip = blobs[blobs.length - 1];
		expect(tip.x).toBeLessThan(1170);
		expect(tip.y).toBeCloseTo(546, 0);
	});

	it("bounds the drawing around every body, the den's shoulders included", () => {
		const scene = new DenScene();
		scene.setInput(input({}, {}, sideDen));
		settle(scene);
		const bounds = gooBounds(scene.frame().blobs);
		expect(bounds).not.toBeNull();
		expect(bounds?.x).toBeLessThan(1170 - 30);
		expect((bounds?.x ?? 0) + (bounds?.width ?? 0)).toBeGreaterThan(1200);
		expect(bounds?.y).toBeLessThan(500 - 30);
	});
});

describe("motion", () => {
	const steps = (scene: DenScene, seconds: number) => {
		for (let elapsed = 0; elapsed < seconds; elapsed += 1 / 60)
			scene.step(1 / 60);
	};

	it("grows the den out of its edge and pops the bots in one by one", () => {
		const scene = new DenScene();
		scene.setInput(input());
		scene.step(1 / 60);
		const start = scene.frame();
		expect(start.den.height).toBeLessThan(topDen.rect.height / 2);
		expect(start.bots.get("a")?.opacity).toBe(0);
		steps(scene, 0.2);
		const first = scene.frame();
		expect(first.bots.get("a")?.opacity ?? 0).toBeGreaterThan(0);
		expect(first.bots.get("b")?.opacity).toBe(0);
		settle(scene);
		expect(scene.frame().bots.get("b")?.opacity).toBe(1);
	});

	it("circles a working bot with moons and keeps animating while it works", () => {
		const scene = new DenScene();
		const working = new Map([["a", "working" as const]]);
		scene.setInput(input({ a: perch() }, { activity: working }));
		settle(scene);
		expect(scene.ambient()).toBe(true);
		const before = scene.frame().blobs.length;
		steps(scene, 0.25);
		const after = scene.frame().blobs;
		expect(after.length).toBe(before);
		const moons = after.filter(
			(blob) => !blob.to && blob.hw < 6 && blob.hw === blob.hh,
		);
		expect(moons.length).toBeGreaterThanOrEqual(3);
		scene.setInput(input({ a: perch() }));
		expect(scene.ambient()).toBe(false);
	});

	it("runs a bead along the den while a bot works there", () => {
		const scene = new DenScene();
		const working = new Map([["a", "working" as const]]);
		scene.setInput(input({}, { activity: working }));
		settle(scene);
		const beadOf = () =>
			scene
				.frame()
				.blobs.find((blob) => !blob.to && Math.abs(blob.hw - 4.6) < 0.01);
		const first = beadOf();
		steps(scene, 0.4);
		const later = beadOf();
		expect(first).toBeDefined();
		expect(later?.x).not.toBeCloseTo(first?.x ?? 0, 0);
	});

	it("beats like a heart while a perched bot waits for its owner", () => {
		const scene = new DenScene();
		const waiting = new Map([["a", "waiting" as const]]);
		scene.setInput(input({ a: perch() }, { activity: waiting }));
		settle(scene);
		const widths: number[] = [];
		for (let frame = 0; frame < 80; frame++) {
			scene.step(1 / 60);
			const body = scene
				.frame()
				.blobs.find(
					(blob) => !blob.to && Math.abs(blob.x - 400) < 1 && blob.hw > 10,
				);
			widths.push(body?.hw ?? 0);
		}
		expect(Math.max(...widths)).toBeGreaterThan(Math.min(...widths) * 1.1);
	});

	it("lets a neck thin and snap back into the den instead of vanishing", () => {
		const scene = new DenScene();
		scene.setInput(input({ a: { kind: "drag", x: 790, y: 70 } }));
		settle(scene);
		scene.setInput(input({ a: { kind: "drag", x: 790, y: 400 } }));
		steps(scene, 0.08);
		const snapping = scene.frame().blobs.filter((blob) => blob.to);
		expect(snapping.length).toBeGreaterThanOrEqual(2);
		const end = snapping[1].to;
		expect(end?.y ?? 0).toBeLessThan(400);
		settle(scene);
		expect(scene.frame().blobs.filter((blob) => blob.to)).toHaveLength(0);
	});

	it("sprays droplets on a landing that merge back", () => {
		const scene = new DenScene();
		scene.setInput(input({ a: { kind: "drag", x: 400, y: 287 } }));
		settle(scene);
		const resting = scene.frame().blobs.length;
		scene.setInput(input({ a: perch() }));
		steps(scene, 0.1);
		expect(scene.frame().blobs.length).toBeGreaterThan(resting + 2);
		settle(scene);
		expect(scene.frame().blobs).toHaveLength(3);
	});

	it("keeps a held bot under the pointer, leaning and trailing as it moves", () => {
		const scene = new DenScene();
		scene.setInput(input());
		settle(scene);
		const gaps: number[] = [];
		let tail = false;
		for (let frame = 0; frame < 60; frame++) {
			const pointer = { x: 790 + frame * 12, y: 60 + frame * 6 };
			scene.setInput(input({ a: { kind: "drag", ...pointer } }));
			scene.step(1 / 60);
			const current = scene.frame();
			const bot = current.bots.get("a");
			if (frame > 20)
				gaps.push(
					Math.hypot((bot?.x ?? 0) - pointer.x, (bot?.y ?? 0) - pointer.y),
				);
			tail ||=
				frame > 20 && current.blobs.some((blob) => blob.to && blob.y > 100);
		}
		expect(Math.max(...gaps)).toBeLessThan(1);
		expect(tail).toBe(true);
		expect(scene.frame().bots.get("a")?.tilt ?? 0).toBeGreaterThan(1);
	});

	it("splashes and spreads its foot when the bot touches down, not when it is let go", () => {
		const scene = new DenScene();
		scene.setInput(input({ a: { kind: "drag", x: 400, y: 520 } }));
		settle(scene);
		const droplets = () =>
			scene
				.frame()
				.blobs.filter((blob) => !blob.to && blob.hw < 5 && blob.hw === blob.hh);
		const foot = () =>
			scene.frame().blobs.find((blob) => blob.hh === 2.5 && blob.hw > 2.5);
		scene.setInput(input({ a: perch() }));
		scene.step(1 / 60);
		expect(droplets()).toHaveLength(0);
		expect(foot()).toBeUndefined();
		let landed = false;
		for (let frame = 0; frame < 60 && !landed; frame++) {
			scene.step(1 / 60);
			landed = droplets().length > 0;
			if (landed)
				expect(
					Math.abs((scene.frame().bots.get("a")?.y ?? 0) - 287),
				).toBeLessThan(6);
		}
		expect(landed).toBe(true);
		settle(scene);
		expect(foot()?.hw).toBeCloseTo(15, 1);
	});

	it("swells the den under a hovered bot instead of popping a bulge out", () => {
		const scene = new DenScene();
		scene.setInput(input());
		settle(scene);
		const bulge = () =>
			scene
				.frame()
				.blobs.find(
					(blob) => !blob.to && blob.x === 784 && blob.hw === blob.hh,
				);
		scene.setInput(input({}, { hoverBot: "a" }));
		scene.step(1 / 60);
		expect(bulge()?.hw ?? 0).toBeLessThan(6);
		settle(scene);
		expect(bulge()?.hw).toBeCloseTo(11, 1);
		scene.setInput(input());
		settle(scene);
		expect(bulge()).toBeUndefined();
	});

	it("draws the moons of a bot that stops working back into it", () => {
		const scene = new DenScene();
		const working = new Map([["a", "working" as const]]);
		scene.setInput(input({ a: perch() }, { activity: working }));
		settle(scene);
		const moons = () =>
			scene
				.frame()
				.blobs.filter(
					(blob) =>
						!blob.to && blob.hw === blob.hh && blob.hw > 0 && blob.hw < 6,
				);
		const busy = moons().length;
		scene.setInput(input({ a: perch() }));
		steps(scene, 0.05);
		expect(moons().length).toBe(busy);
		settle(scene);
		expect(moons()).toHaveLength(0);
	});

	it("bulges the den toward a pointer close to it", () => {
		const scene = new DenScene();
		scene.setInput(input());
		settle(scene);
		scene.setInput(input({}, { pointer: { x: 830, y: 52 } }));
		settle(scene);
		const bump = scene.frame().blobs[1];
		expect(bump).toBeDefined();
		expect(bump.x).toBeGreaterThan(800);
		scene.setInput(input({}, { pointer: { x: 200, y: 700 } }));
		settle(scene);
		expect(scene.frame().blobs).toHaveLength(1);
	});
});
