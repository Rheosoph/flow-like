import { describe, expect, test } from "bun:test";
import {
	getEventGuide,
	getEventSections,
	getSectionGuidance,
	isTriggerSection,
} from "./event-sections";
import type { IEvent } from "./schema/flow/event";

const baseEvent = (overrides: Partial<IEvent> = {}): IEvent =>
	({
		active: true,
		board_id: "board-1",
		config: [],
		created_at: { nanos_since_epoch: 0, secs_since_epoch: 0 },
		description: "",
		event_type: "cron",
		event_version: [1, 0, 0],
		id: "evt-1",
		name: "Nightly import",
		node_id: "node-1",
		priority: 0,
		updated_at: { nanos_since_epoch: 0, secs_since_epoch: 0 },
		variables: {},
		...overrides,
	}) as IEvent;

describe("getEventSections", () => {
	test("leads with the type-specific section, then the shared ones", () => {
		// email is not split, so it still renders as one lump.
		const sections = getEventSections(baseEvent({ event_type: "email" }));
		expect(sections[0].id).toBe("trigger");
		expect(sections.map((s) => s.id)).toEqual([
			"trigger",
			"flow",
			"inputs",
			"variables",
			"release",
			"canary",
			"quality",
			"history",
			"identity",
		]);
	});

	test("labels the lump section per event type", () => {
		expect(getEventSections(baseEvent({ event_type: "email" }))[0].label).toBe(
			"Mailbox",
		);
		expect(
			getEventSections(baseEvent({ event_type: "deeplink" }))[0].label,
		).toBe("Deep link");
	});

	test("falls back for unknown event types rather than rendering nothing", () => {
		const sections = getEventSections(baseEvent({ event_type: "brand_new" }));
		expect(sections[0].label).toBe("Configuration");
		expect(sections).toHaveLength(9);
	});

	test("page-target events lead with Flow & target, where the page lives", () => {
		const sections = getEventSections(
			baseEvent({ event_type: "quick_action", default_page_id: "page-1" }),
		);
		expect(sections[0].label).toBe("Flow & target");
	});

	test("a page event counts as bound without an entry node", () => {
		const page = baseEvent({ default_page_id: "page-1", node_id: "" });
		const bind = getEventGuide(page).find((s) => s.id === "bind-flow");
		expect(bind?.auto?.({}, page)).toBe(true);
	});

	test("split types expose one section per group instead of one lump", () => {
		const discord = getEventSections(baseEvent({ event_type: "discord" }));
		expect(discord.map((s) => s.id)).toEqual([
			"connection",
			"permissions",
			"channels",
			"behaviour",
			"flow",
			"inputs",
			"variables",
			"release",
			"canary",
			"quality",
			"history",
			"identity",
		]);
		expect(
			getEventSections(baseEvent())
				.map((s) => s.id)
				.slice(0, 2),
		).toEqual(["schedule", "runtime"]);
	});

	test("page-target events get no trigger section at all", () => {
		// There is no configInterfaces["page"], so a trigger section for a page
		// event is a tab that can never render anything.
		const sections = getEventSections(
			baseEvent({ event_type: "quick_action", default_page_id: "page-1" }),
		);
		expect(sections.map((s) => s.id)).toEqual([
			"flow",
			"inputs",
			"variables",
			"release",
			"canary",
			"history",
			"identity",
		]);
	});

	test("a page-targeted split type also drops its trigger sections", () => {
		const sections = getEventSections(
			baseEvent({ event_type: "discord", default_page_id: "page-1" }),
		);
		expect(sections[0].id).toBe("flow");
		expect(sections.some((s) => s.id === "connection")).toBe(false);
	});

	test("unsplit types keep the single trigger section", () => {
		for (const type of ["email", "deeplink", "quick_action", "generic_form"]) {
			expect(getEventSections(baseEvent({ event_type: type }))[0].id).toBe(
				"trigger",
			);
		}
	});

	test("split types cover every event type whose component takes a section", () => {
		// Keep this in step with the components that destructure `section`.
		const split = [
			"cron",
			"api",
			"discord",
			"telegram",
			"simple_chat",
			"daemon",
			"rest",
			"mcp",
		];
		for (const type of split) {
			const first = getEventSections(baseEvent({ event_type: type }))[0];
			expect({ type, id: first.id }).not.toEqual({ type, id: "trigger" });
		}
	});
});

describe("isTriggerSection", () => {
	test("shared sections are not trigger sections", () => {
		for (const id of [
			"flow",
			"inputs",
			"variables",
			"release",
			"canary",
			"quality",
			"history",
			"identity",
		]) {
			expect(isTriggerSection(id)).toBe(false);
		}
	});

	test("the lump and every split id are trigger sections", () => {
		for (const id of ["trigger", "connection", "permissions", "schedule"]) {
			expect(isTriggerSection(id)).toBe(true);
		}
	});

	test("every section a split type declares routes to the config component", () => {
		const discord = getEventSections(baseEvent({ event_type: "discord" }));
		const triggerIds = discord.filter((s) => isTriggerSection(s.id));
		expect(triggerIds).toHaveLength(4);
	});
});

describe("getEventGuide", () => {
	test("derives ticks from config for automatic steps", () => {
		const event = baseEvent();
		const steps = getEventGuide(event);
		const when = steps.find((s) => s.id === "when");
		expect(when?.auto?.({}, event)).toBe(false);
		expect(when?.auto?.({ expression: "0 9 * * 1-5" }, event)).toBe(true);
	});

	test("treats an empty string as unset", () => {
		const event = baseEvent();
		const when = getEventGuide(event).find((s) => s.id === "when");
		expect(when?.auto?.({ expression: "   " }, event)).toBe(false);
	});

	test("marks portal work as external and names where to do it", () => {
		const steps = getEventGuide(baseEvent({ event_type: "discord" }));
		const intent = steps.find((s) => s.id === "message-content");
		expect(intent?.external).toBe(true);
		expect(intent?.where).toContain("Privileged Gateway Intents");
		expect(intent?.section).toBeUndefined();
	});

	test("steps without auto are user-confirmed", () => {
		const steps = getEventGuide(baseEvent());
		expect(steps.find((s) => s.id === "first-run")?.auto).toBeUndefined();
	});

	test("every event type gets a guide, known or not", () => {
		expect(
			getEventGuide(baseEvent({ event_type: "brand_new" })).length,
		).toBeGreaterThan(0);
	});

	test("flow binding ticks only when both board and node are set", () => {
		const bind = getEventGuide(baseEvent()).find((s) => s.id === "bind-flow");
		expect(bind?.auto?.({}, baseEvent())).toBe(true);
		expect(bind?.auto?.({}, baseEvent({ node_id: "" }))).toBe(false);
	});

	test("case keys tick from the correlation mappings", () => {
		const step = getEventGuide(baseEvent()).find((s) => s.id === "case-keys");
		expect(step?.auto?.({}, baseEvent())).toBe(false);
		expect(
			step?.auto?.(
				{},
				baseEvent({ correlation_mappings: { order_id: "order.id" } }),
			),
		).toBe(true);
	});
});

describe("getSectionGuidance", () => {
	test("returns type-specific guidance for an unsplit trigger section", () => {
		const guidance = getSectionGuidance(
			baseEvent({ event_type: "email" }),
			"trigger",
		);
		expect(guidance?.mistake).toContain("newsletter");
	});

	test("returns shared guidance for shared sections", () => {
		expect(getSectionGuidance(baseEvent(), "variables")?.mistake).toContain(
			"exposed",
		);
	});

	test("returns null rather than an empty callout for unknown types", () => {
		expect(
			getSectionGuidance(baseEvent({ event_type: "brand_new" }), "trigger"),
		).toBeNull();
	});

	test("every declared section has guidance, so no section renders bare", () => {
		for (const type of [
			"discord",
			"cron",
			"telegram",
			"api",
			"simple_chat",
			"daemon",
			"rest",
			"mcp",
		]) {
			const event = baseEvent({ event_type: type });
			for (const section of getEventSections(event)) {
				expect(getSectionGuidance(event, section.id)).not.toBeNull();
			}
		}
	});

	test("split guidance beats the whole-type fallback", () => {
		const event = baseEvent({ event_type: "discord" });
		expect(getSectionGuidance(event, "permissions")?.mistake).toContain(
			"MessageContent",
		);
		expect(getSectionGuidance(event, "channels")?.mistake).toContain("empty");
	});

	test("page events get bootstrap-aware canary guidance, other sections shared", () => {
		const page = baseEvent({ event_type: "page", default_page_id: "page-1" });
		expect(getSectionGuidance(page, "canary")?.mistake).toContain("reload");
		expect(getSectionGuidance(page, "variables")?.mistake).toContain("exposed");
		for (const section of getEventSections(page)) {
			expect(getSectionGuidance(page, section.id)).not.toBeNull();
		}
	});
});

describe("guide steps point at sections that exist", () => {
	test("no step references a section the event does not have", () => {
		for (const type of [
			"cron",
			"api",
			"discord",
			"telegram",
			"simple_chat",
			"email",
			"daemon",
			"mcp",
			"rest",
			"deeplink",
			"unknown_type",
		]) {
			const event = baseEvent({ event_type: type });
			const ids = new Set(getEventSections(event).map((s) => s.id));
			for (const step of getEventGuide(event)) {
				if (!step.section) continue;
				expect({ type, step: step.id, section: step.section }).toEqual({
					type,
					step: step.id,
					section: ids.has(step.section) ? step.section : "MISSING SECTION",
				});
			}
		}
	});
});
