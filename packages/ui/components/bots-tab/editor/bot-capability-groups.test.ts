import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import bots from "../../../../locales/locales/en/bots.json";
import {
	BOT_CAPABILITIES,
	DEVICE_UNAVAILABLE_BOT_CAPABILITIES,
	botCapabilityUnavailableReason,
	isBotCapabilityAvailable,
} from "../../../state/backend-state/bot-state";
import {
	BOT_CAPABILITY_GROUPS,
	BOT_CAPABILITY_ICONS,
} from "./bot-capability-groups";

const NEW_CAPABILITIES = [
	"documents",
	"barcodes",
	"data_connections",
	"knowledge",
	"connectors",
	"channels",
	"skills",
	"image_generation",
	"delegation",
] as const;

function rustWireNames(): string[] {
	const source = readFileSync(
		resolve(import.meta.dir, "../../../../types/contracts/src/bot.rs"),
		"utf8",
	);
	const body = source.slice(source.indexOf("pub fn as_str(self)"));
	const match = body.slice(0, body.indexOf("\n    }\n"));
	return [...match.matchAll(/=> "([a-z_]+)",/g)].map((hit) => hit[1]);
}

describe("capability list", () => {
	test("matches the Rust contracts in wire name and order", () => {
		expect([...BOT_CAPABILITIES] as string[]).toEqual(rustWireNames());
	});

	test("puts the new capabilities right after memory", () => {
		const memory = BOT_CAPABILITIES.indexOf("memory");
		expect(
			BOT_CAPABILITIES.slice(memory + 1, memory + 1 + NEW_CAPABILITIES.length),
		).toEqual([...NEW_CAPABILITIES]);
	});

	test("every capability has one editor card, an icon and copy", () => {
		const shown = BOT_CAPABILITY_GROUPS.flatMap((group) => group.capabilities);
		const copy = bots.capabilities as Record<
			string,
			{ label?: string; description?: string }
		>;
		for (const capability of BOT_CAPABILITIES) {
			if (capability === "flowpilot") continue;
			expect(shown.filter((id) => id === capability)).toHaveLength(1);
			expect(BOT_CAPABILITY_ICONS[capability]).toBeDefined();
			expect(copy[capability]?.label).toBeTruthy();
			expect(copy[capability]?.description).toBeTruthy();
		}
	});

	test("the new capabilities are allowed on the server and desktop, and on a device unless it can't serve them yet", () => {
		for (const capability of NEW_CAPABILITIES) {
			for (const location of ["server", "desktop"] as const) {
				expect(isBotCapabilityAvailable(capability, location)).toBe(true);
			}
			expect(isBotCapabilityAvailable(capability, "device")).toBe(
				!DEVICE_UNAVAILABLE_BOT_CAPABILITIES.includes(capability),
			);
		}
	});

	test("a device can't hold encryption, and says why", () => {
		expect([...DEVICE_UNAVAILABLE_BOT_CAPABILITIES]).toEqual(["encryption"]);
		for (const capability of DEVICE_UNAVAILABLE_BOT_CAPABILITIES) {
			expect(botCapabilityUnavailableReason(capability, "device")).toBe(
				"device",
			);
			expect(botCapabilityUnavailableReason(capability, "desktop")).toBeNull();
		}
		expect(botCapabilityUnavailableReason("command_line", "device")).toBeNull();
		expect(botCapabilityUnavailableReason("command_line", "server")).toBe(
			"local",
		);
		expect(bots.editor.abilities.notOnDevice).toBeTruthy();
	});
});
