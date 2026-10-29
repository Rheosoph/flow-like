import { describe, expect, test } from "vitest";
import { upsertAppEvent } from "./event-tools";

function fixture(
	options: {
		routeFails?: boolean;
		cancelled?: boolean;
		responseLost?: boolean;
		triggerError?: string;
		/** The flag the desktop rejects a save with when a prompt before the write is refused. */
		promptRefusal?: Record<string, boolean>;
	} = {},
) {
	const saved: Record<string, unknown>[] = [];
	const routes: string[] = [];
	const backend = {
		eventState: {
			getEvents: async () => [],
			getEventsAuthoritative: async () => [],
			getEvent: async () => {
				throw new Error("not found");
			},
			upsertEvent: async (_app: string, event: Record<string, unknown>) => {
				if (options.promptRefusal)
					throw Object.assign(
						new Error("prompt refused before the write"),
						options.promptRefusal,
					);
				saved.push(event);
				if (options.responseLost) throw new Error("response lost after write");
				return options.triggerError
					? { ...event, trigger_error: options.triggerError }
					: event;
			},
		},
		boardState: {},
		routeState: {
			setRoute: async (_app: string, route: string) => {
				if (options.routeFails) throw new Error("route store unavailable");
				routes.push(route);
			},
		},
	} as unknown as Parameters<typeof upsertAppEvent>[0];
	return {
		backend,
		saved,
		routes,
		context: {
			assertActive() {
				if (options.cancelled) throw new Error("cancelled");
			},
			referenceApp() {},
		},
	};
}

describe("Event provisioning receipts", () => {
	test("lost write responses preserve the attempted Event ID and unknown outcome", async () => {
		const f = fixture({ responseLost: true });
		const result = await upsertAppEvent(
			f.backend,
			{ app_id: "app", name: "Home", page_id: "page", route: "home" },
			f.context,
		);
		expect(result).toMatchObject({
			status: "unknown",
			code: "event_write_outcome_unknown",
			event_id: f.saved[0].id,
			provisioning: { event_saved: "unknown", route_applied: false },
		});
		expect(f.routes).toHaveLength(0);
	});
	test.each([
		[
			"dismissed trigger consent or token prompt",
			"isLocalSinkConsentDismissed",
		],
		["refused computer automation approval", "isRpaConsentError"],
	])(
		"a %s reports a cancelled write, not an unknown outcome",
		async (_, flag) => {
			const f = fixture({ promptRefusal: { [flag]: true } });
			const result = await upsertAppEvent(
				f.backend,
				{ app_id: "app", name: "Home", page_id: "page", route: "home" },
				f.context,
			);
			expect(result).toMatchObject({
				status: "error",
				code: "event_write_cancelled",
				provisioning: { event_saved: false, route_applied: false },
			});
			expect(result.recovery).toBeUndefined();
			expect(f.routes).toHaveLength(0);
		},
	);
	test("a refusal flag that is not true leaves the outcome unknown", async () => {
		const f = fixture({
			promptRefusal: { isLocalSinkConsentDismissed: false },
		});
		const result = await upsertAppEvent(
			f.backend,
			{ app_id: "app", name: "Home", page_id: "page" },
			f.context,
		);
		expect(result).toMatchObject({
			status: "unknown",
			code: "event_write_outcome_unknown",
			provisioning: { event_saved: "unknown" },
		});
	});
	test("route failure is partial and keeps the persisted Event identity", async () => {
		const f = fixture({ routeFails: true });
		const result = await upsertAppEvent(
			f.backend,
			{ app_id: "app", name: "Home", page_id: "page", route: "home" },
			f.context,
		);
		expect(result.status).toBe("partial");
		expect(result.code).toBe("event_route_incomplete");
		expect(result.event_id).toBe(f.saved[0].id);
		expect(result.provisioning).toEqual({
			event_saved: true,
			route_applied: false,
		});
	});
	test("a saved Event whose trigger on this computer failed reports saved, not an unknown outcome", async () => {
		const f = fixture({
			triggerError:
				"Event was saved, but its trigger on this computer was not registered: no token",
		});
		const result = await upsertAppEvent(
			f.backend,
			{ app_id: "app", name: "Home", page_id: "page", route: "home" },
			f.context,
		);
		expect(result).toMatchObject({
			status: "saved",
			trigger_error:
				"Event was saved, but its trigger on this computer was not registered: no token",
			event_id: f.saved[0].id,
			provisioning: { event_saved: true, route_applied: true },
		});
		expect(result.code).toBeUndefined();
		expect(f.routes).toEqual(["/home"]);
	});
	test("build host reserves an inactive Event before its first write", async () => {
		const f = fixture();
		const result = await upsertAppEvent(
			f.backend,
			{ app_id: "app", name: "Home", page_id: "page", active: false },
			{ ...f.context, reservedEventId: "reserved" },
		);
		expect(result.status).toBe("ok");
		expect(f.saved[0].id).toBe("reserved");
		expect(f.saved[0].active).toBe(false);
	});
	test("ordinary updates do not reinterpret a missing Event as creation", async () => {
		const f = fixture();
		const result = await upsertAppEvent(
			f.backend,
			{ app_id: "app", name: "Home", page_id: "page", event_id: "missing" },
			f.context,
		);
		expect(result.status).toBe("error");
		expect(f.saved).toHaveLength(0);
	});
	test("expired requests cannot persist an Event", async () => {
		const f = fixture({ cancelled: true });
		const result = await upsertAppEvent(
			f.backend,
			{ app_id: "app", name: "Home", page_id: "page" },
			f.context,
		);
		expect(result.status).toBe("error");
		expect(f.saved).toHaveLength(0);
	});
});
