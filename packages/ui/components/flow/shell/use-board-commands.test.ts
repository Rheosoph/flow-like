import { describe, expect, test } from "bun:test";
import {
	type IBoardCommand,
	type IBoardCommandSurface,
	commandsFor,
	formatShortcut,
} from "./use-board-commands";

const command = (
	id: string,
	surface: IBoardCommandSurface,
	when?: boolean,
): IBoardCommand => ({ id, title: id, surface, when, run: () => {} });

describe("commandsFor", () => {
	test("returns only the commands that declared this surface", () => {
		const commands = [
			command("explorer", "rail"),
			command("templates", "editor"),
			command("inspector", "rail-bottom"),
			command("find", "palette"),
		];

		expect(commandsFor(commands, "rail").map((c) => c.id)).toEqual([
			"explorer",
		]);
		expect(commandsFor(commands, "editor").map((c) => c.id)).toEqual([
			"templates",
		]);
		expect(commandsFor(commands, "palette").map((c) => c.id)).toEqual(["find"]);
	});

	test("drops commands whose when-clause is false", () => {
		const commands = [
			command("layer-up", "rail-bottom", false),
			command("inspector", "rail-bottom", true),
			command("flowpilot", "rail-bottom"),
		];
		expect(commandsFor(commands, "rail-bottom").map((c) => c.id)).toEqual([
			"inspector",
			"flowpilot",
		]);
	});

	test("preserves registry order, which is the order surfaces render in", () => {
		const commands = [
			command("b", "rail"),
			command("a", "rail"),
			command("c", "rail"),
		];
		expect(commandsFor(commands, "rail").map((c) => c.id)).toEqual([
			"b",
			"a",
			"c",
		]);
	});

	test("every command reaches exactly one surface", () => {
		// The regression this guards: Templates and Auto Layout were registered as
		// commands, rendered by nothing, and reachable only by searching for them.
		const surfaces: IBoardCommandSurface[] = [
			"rail",
			"rail-bottom",
			"editor",
			"status",
			"palette",
		];
		const commands = [
			command("explorer", "rail"),
			command("templates", "editor"),
			command("auto-layout", "editor"),
			command("inspector", "rail-bottom"),
		];

		for (const entry of commands) {
			const hits = surfaces.filter((surface) =>
				commandsFor(commands, surface).some((c) => c.id === entry.id),
			);
			expect(hits).toHaveLength(1);
		}
	});
});

describe("formatShortcut", () => {
	test("renders a chord for the current platform", () => {
		const rendered = formatShortcut("mod+shift+p");
		expect(rendered).toMatch(/P$/);
		expect(rendered.includes("⌘") || rendered.includes("Ctrl")).toBe(true);
	});
});
