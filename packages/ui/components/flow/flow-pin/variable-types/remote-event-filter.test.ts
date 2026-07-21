import { describe, expect, test } from "vitest";
import type { IRemoteEvent } from "../../../../state/backend-state/types";
import {
	filterRemoteEventsForNode,
	remoteEventTypesForNode,
} from "./remote-event-filter";

const events: IRemoteEvent[] = [
	{ id: "chat", name: "Assistant", event_type: "simple_chat" },
	{ id: "api", name: "Orders API", event_type: "rest" },
	{ id: "mcp", name: "Tools", event_type: "mcp" },
];

describe("remote event filtering", () => {
	test("offers only REST events to Call Remote API", () => {
		expect(filterRemoteEventsForNode(events, "call_remote_api")).toEqual([
			events[1],
		]);
	});

	test("offers only simple chat events to Call Remote Chat", () => {
		expect(filterRemoteEventsForNode(events, "call_remote_chat")).toEqual([
			events[0],
		]);
	});

	test("leaves the legacy and unknown nodes unfiltered", () => {
		expect(filterRemoteEventsForNode(events, "call_remote_event")).toEqual(
			events,
		);
		expect(
			filterRemoteEventsForNode(events, "third_party_remote_call"),
		).toEqual(events);
		expect(remoteEventTypesForNode("call_remote_event")).toBeUndefined();
	});
});
