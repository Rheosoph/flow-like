import { describe, expect, test } from "bun:test";
import {
	activePageEventCandidates,
	classifyAppEventInterface,
	consumerToolForEventKind,
	resolveOpenAppPageEvent,
	resolveOpenAppPageRequest,
} from "./app-event-interface";

const event = (
	overrides: Partial<{
		id: string;
		name: string;
		event_type: string;
		active: boolean;
		default_page_id: string | null;
		route: string | null;
	}> = {},
) => ({
	id: "event-1",
	name: "Briefing",
	event_type: "page",
	active: true,
	default_page_id: "page-1",
	route: "/briefing",
	...overrides,
});

describe("FlowPilot app interface classification", () => {
	test("uses one page invariant for inventory and exact opening", () => {
		const listed = event();
		expect(classifyAppEventInterface(listed)).toBe("page");
		expect(activePageEventCandidates([listed])).toEqual([
			{
				event_id: "event-1",
				name: "Briefing",
				page_id: "page-1",
				route: "/briefing",
			},
		]);
		expect(resolveOpenAppPageEvent([listed], listed.id)).toEqual({
			ok: true,
			event: listed,
		});
	});

	test("does not treat a page discriminator without a real target as embeddable", () => {
		for (const defaultPageId of [null, "", "   ", " page-1 "]) {
			const broken = event({ default_page_id: defaultPageId });
			expect(classifyAppEventInterface(broken)).toBe("unavailable");
			expect(resolveOpenAppPageEvent([broken], broken.id)).toEqual({
				ok: false,
				code: "page_target_missing",
				actual_kind: "unavailable",
				relist_required: true,
			});
		}
	});

	test("returns one exact callable consumer and none for malformed pages", () => {
		expect(consumerToolForEventKind("chat")).toBe("call_app_chat");
		expect(consumerToolForEventKind("page")).toBe("open_app_page");
		expect(consumerToolForEventKind("headless")).toBe("call_app_event");
		expect(consumerToolForEventKind("unavailable")).toBeUndefined();
	});

	test("chat takes precedence over stale page metadata", () => {
		const chat = event({ event_type: "simple_chat" });
		expect(classifyAppEventInterface(chat)).toBe("chat");
		expect(resolveOpenAppPageEvent([chat], chat.id)).toEqual({
			ok: false,
			code: "event_interface_changed",
			actual_kind: "chat",
			relist_required: true,
		});
	});

	test("separates inactive, changed, missing, and no-page outcomes", () => {
		expect(
			resolveOpenAppPageEvent([event({ active: false })], "event-1"),
		).toEqual({
			ok: false,
			code: "event_inactive",
			actual_kind: "page",
			relist_required: true,
		});
		expect(
			resolveOpenAppPageEvent(
				[event({ event_type: "quick_action", default_page_id: null })],
				"event-1",
			),
		).toEqual({
			ok: false,
			code: "event_interface_changed",
			actual_kind: "headless",
			relist_required: true,
		});
		expect(resolveOpenAppPageEvent([event()], "missing")).toEqual({
			ok: false,
			code: "event_not_found",
			relist_required: true,
		});
		expect(resolveOpenAppPageEvent([], undefined)).toEqual({
			ok: false,
			code: "no_page_event",
			relist_required: false,
		});
	});

	test("canonicalizes a page id only when it maps to exactly one active page event", () => {
		const page = event();
		expect(resolveOpenAppPageEvent([page], "page-1")).toEqual({
			ok: true,
			event: page,
			canonicalized_from: "page_id",
		});
		expect(
			resolveOpenAppPageEvent(
				[page, event({ id: "event-2", name: "Duplicate" })],
				"page-1",
			),
		).toEqual({
			ok: false,
			code: "event_not_found",
			relist_required: true,
		});
	});

	test("keeps an exact refreshed capability authoritative over a conflicting bulk snapshot", async () => {
		const exact = event({
			event_type: "quick_action",
			default_page_id: null,
		});
		const staleBulkCopy = event();
		const sibling = event({
			id: "event-2",
			name: "Overview",
			default_page_id: "page-2",
		});
		const lookup = await resolveOpenAppPageRequest(
			exact.id,
			async () => exact,
			async () => [staleBulkCopy, sibling],
		);
		expect(lookup).toEqual({
			status: "resolved",
			resolution: {
				ok: false,
				code: "event_interface_changed",
				actual_kind: "headless",
				relist_required: true,
			},
			candidate_events: [sibling],
		});
	});

	test("distinguishes an unavailable inventory from an authoritative not-found result", async () => {
		const unavailable = await resolveOpenAppPageRequest(
			"missing",
			async () => {
				throw new Error("lookup unavailable");
			},
			async () => {
				throw new Error("inventory unavailable");
			},
		);
		expect(unavailable).toEqual({ status: "inventory_unavailable" });

		const notFound = await resolveOpenAppPageRequest(
			"missing",
			async () => {
				throw new Error("not found");
			},
			async () => [event()],
		);
		expect(notFound).toEqual({
			status: "resolved",
			resolution: {
				ok: false,
				code: "event_not_found",
				relist_required: true,
			},
			candidate_events: [event()],
		});
	});
});
