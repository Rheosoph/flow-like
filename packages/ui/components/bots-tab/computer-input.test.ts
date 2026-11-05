import { describe, expect, test } from "bun:test";
import {
	buttonName,
	keyToInput,
	mapToFrame,
	wheelToInput,
} from "./computer-input";

const frame = { width: 1280, height: 800 };
const key = (
	k: string,
	mods: Partial<
		Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>
	> = {},
) => ({
	key: k,
	ctrlKey: false,
	altKey: false,
	shiftKey: false,
	metaKey: false,
	...mods,
});

describe("mapToFrame", () => {
	test("a box of the frame's own ratio scales linearly", () => {
		const box = { left: 100, top: 50, width: 640, height: 400 };
		expect(mapToFrame({ clientX: 100, clientY: 50 }, box, frame)).toEqual({
			x: 0,
			y: 0,
		});
		expect(mapToFrame({ clientX: 420, clientY: 250 }, box, frame)).toEqual({
			x: 640,
			y: 400,
		});
	});

	test("a wide box letterboxes left and right", () => {
		const box = { left: 0, top: 0, width: 1600, height: 400 };
		// drawn 640x400 centered: left bar is 480 px
		expect(mapToFrame({ clientX: 100, clientY: 200 }, box, frame)).toBeNull();
		expect(
			mapToFrame({ clientX: 480 + 320, clientY: 200 }, box, frame),
		).toEqual({ x: 640, y: 400 });
	});

	test("a tall box letterboxes top and bottom", () => {
		const box = { left: 0, top: 0, width: 640, height: 800 };
		// drawn 640x400, top bar 200
		expect(mapToFrame({ clientX: 10, clientY: 100 }, box, frame)).toBeNull();
		expect(mapToFrame({ clientX: 320, clientY: 400 }, box, frame)).toEqual({
			x: 640,
			y: 400,
		});
	});

	test("clamping keeps drags that leave the picture on its edge", () => {
		const box = { left: 0, top: 0, width: 640, height: 400 };
		expect(
			mapToFrame({ clientX: 9999, clientY: -5 }, box, frame, true),
		).toEqual({ x: 1279, y: 0 });
	});

	test("degenerate sizes map nowhere", () => {
		expect(
			mapToFrame(
				{ clientX: 1, clientY: 1 },
				{ left: 0, top: 0, width: 0, height: 0 },
				frame,
			),
		).toBeNull();
		expect(
			mapToFrame(
				{ clientX: 1, clientY: 1 },
				{ left: 0, top: 0, width: 10, height: 10 },
				{ width: 0, height: 0 },
			),
		).toBeNull();
	});
});

describe("keyToInput", () => {
	test("typed characters are text, including shifted ones and space", () => {
		expect(keyToInput(key("a"))).toEqual({ type: "text", text: "a" });
		expect(keyToInput(key("A", { shiftKey: true }))).toEqual({
			type: "text",
			text: "A",
		});
		expect(keyToInput(key(" "))).toEqual({ type: "text", text: " " });
	});

	test("named keys are chords", () => {
		expect(keyToInput(key("Enter"))).toEqual({ type: "key", chord: "Enter" });
		expect(keyToInput(key("ArrowLeft"))).toEqual({
			type: "key",
			chord: "ArrowLeft",
		});
		expect(keyToInput(key("F5"))).toEqual({ type: "key", chord: "F5" });
		expect(keyToInput(key("Tab", { shiftKey: true }))).toEqual({
			type: "key",
			chord: "Shift+Tab",
		});
	});

	test("shortcuts are chords with modifiers in a fixed order", () => {
		expect(keyToInput(key("c", { ctrlKey: true }))).toEqual({
			type: "key",
			chord: "Control+c",
		});
		expect(keyToInput(key("A", { ctrlKey: true, shiftKey: true }))).toEqual({
			type: "key",
			chord: "Control+Shift+a",
		});
		expect(keyToInput(key("v", { metaKey: true }))).toEqual({
			type: "key",
			chord: "Meta+v",
		});
	});

	test("bare modifiers and unknown keys send nothing", () => {
		expect(keyToInput(key("Shift", { shiftKey: true }))).toBeNull();
		expect(keyToInput(key("Control", { ctrlKey: true }))).toBeNull();
		expect(keyToInput(key("Dead"))).toBeNull();
		expect(keyToInput(key("MediaPlayPause", { ctrlKey: true }))).toBeNull();
	});
});

describe("pointer helpers", () => {
	test("wheel deltas become a rounded scroll at the pointer", () => {
		expect(wheelToInput({ deltaX: 0, deltaY: 99.6 }, { x: 5, y: 6 })).toEqual({
			type: "scroll",
			x: 5,
			y: 6,
			dx: 0,
			dy: 100,
		});
	});

	test("buttons", () => {
		expect([0, 1, 2].map(buttonName)).toEqual(["left", "middle", "right"]);
	});
});
