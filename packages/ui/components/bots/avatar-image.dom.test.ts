// @vitest-environment happy-dom

import { describe, expect, it } from "vitest";
import { botAvatarSvg } from "./avatar-image";

describe("botAvatarSvg", () => {
	it("draws a still, standalone square tile of the look", () => {
		const svg = botAvatarSvg({
			kind: "pip",
			frame: "circle",
			mute_notifications: true,
		});
		expect(svg.startsWith('<svg xmlns="http://www.w3.org/2000/svg"')).toBe(
			true,
		);
		expect(svg.endsWith("</svg>")).toBe(true);
		expect(svg).toContain('width="256"');
		expect(svg).toContain('rx="0"');
		const parsed = new DOMParser().parseFromString(svg, "image/svg+xml");
		expect(parsed.documentElement.tagName.toLowerCase()).toBe("svg");
		expect(parsed.querySelector("parsererror")).toBeNull();
	});
});
