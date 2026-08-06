import { describe, expect, test } from "bun:test";
import {
	WILDCARD_EVENT,
	firstEventAction,
	resolveEventActions,
} from "./event-handlers";
import type { Action } from "./types";

const action = (name: string): Action => ({ name, context: {} });

describe("resolveEventActions", () => {
	test("prefers the exact named event and preserves action order", () => {
		const result = resolveEventActions(
			{ save: [action("workflow_event"), action("navigate_page")] },
			"save",
			[action("legacy")],
		);

		expect(result.source).toBe("event");
		expect(result.actions.map((item) => item.name)).toEqual([
			"workflow_event",
			"navigate_page",
		]);
	});

	test("uses a wildcard handler when no exact event is configured", () => {
		const result = resolveEventActions(
			{ [WILDCARD_EVENT]: [action("external_link")] },
			"open",
			[action("legacy")],
		);

		expect(result.source).toBe("wildcard");
		expect(result.actions[0]?.name).toBe("external_link");
	});

	test("an explicit empty event suppresses wildcard and legacy fallbacks", () => {
		const result = resolveEventActions(
			{
				open: [],
				[WILDCARD_EVENT]: [action("wildcard")],
			},
			"open",
			[action("legacy")],
		);

		expect(result).toEqual({ actions: [], source: "event" });
	});

	test("falls back to only the formerly executable first legacy action", () => {
		const result = resolveEventActions(undefined, "change", [
			action("first"),
			action("previously-inert"),
		]);

		expect(result.source).toBe("legacy");
		expect(result.actions.map((item) => item.name)).toEqual(["first"]);
	});

	test("can disable legacy fallback for newly actionable events", () => {
		const result = resolveEventActions(
			undefined,
			"viewportChange",
			[action("legacy")],
			{ legacyFallback: false },
		);

		expect(result).toEqual({ actions: [], source: "none" });
	});

	test("events opting out of the wildcard need their own handler", () => {
		const handlers = { "*": [action("wildcard")] };

		expect(
			resolveEventActions(handlers, "input", undefined, {
				wildcardFallback: false,
			}),
		).toEqual({ actions: [], source: "none" });

		expect(
			resolveEventActions(handlers, "change", undefined).actions.map(
				(item) => item.name,
			),
		).toEqual(["wildcard"]);

		expect(
			resolveEventActions({ ...handlers, input: [action("typed")] }, "input", [
				action("legacy"),
			]).actions.map((item) => item.name),
		).toEqual(["typed"]);
	});

	test("returns the first resolved action for upload target discovery", () => {
		expect(
			firstEventAction({ change: [action("workflow_event")] }, "change", [
				action("legacy"),
			])?.name,
		).toBe("workflow_event");
	});
});
