import { describe, expect, spyOn, test } from "bun:test";
import type { IEvent } from "../../../lib/schema/flow/event";
import { saveCreatedEventRoute } from "./create-event-route";

const UI_TYPES = new Set(["quick_action"]);
const saved = (patch: Partial<IEvent>) =>
	({ id: "evt_1", event_type: "cron", ...patch }) as IEvent;

function routes() {
	const calls: Array<[string, string, string]> = [];
	return {
		calls,
		setRoute: async (appId: string, path: string, eventId: string) => {
			calls.push([appId, path, eventId]);
			return { path, eventId };
		},
	};
}

describe("saveCreatedEventRoute", () => {
	test("a UI event gets its normalized path, pointing at the saved id", async () => {
		const state = routes();
		const done = await saveCreatedEventRoute(
			state,
			"app",
			saved({ event_type: "quick_action", id: "evt_saved" }),
			"dash/",
			UI_TYPES,
		);
		expect(done).toBe(true);
		expect(state.calls).toEqual([["app", "/dash", "evt_saved"]]);
	});

	test("a page-target event gets a route even when its type is not a UI type", async () => {
		const state = routes();
		await saveCreatedEventRoute(
			state,
			"app",
			saved({ default_page_id: "page_1" }),
			"/home",
			UI_TYPES,
		);
		expect(state.calls).toEqual([["app", "/home", "evt_1"]]);
	});

	test("an event that is neither gets no route", async () => {
		const state = routes();
		expect(
			await saveCreatedEventRoute(state, "app", saved({}), "/x", UI_TYPES),
		).toBe(false);
		expect(state.calls).toEqual([]);
	});

	test("a failing route write is reported, not thrown", async () => {
		const log = spyOn(console, "error").mockImplementation(() => undefined);
		const done = await saveCreatedEventRoute(
			{
				setRoute: async () => {
					throw new Error("conflict");
				},
			},
			"app",
			saved({ event_type: "quick_action" }),
			"/x",
			UI_TYPES,
		);
		expect(done).toBe(false);
		expect(log).toHaveBeenCalled();
		log.mockRestore();
	});
});
