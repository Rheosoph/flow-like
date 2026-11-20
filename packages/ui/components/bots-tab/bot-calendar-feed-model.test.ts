import { describe, expect, test } from "bun:test";
import {
	FEED_DRAFT,
	feedCalendarName,
	feedFlags,
	feedTargets,
} from "./bot-calendar-feed-model";

const feed = {
	url: "https://api.example.com/api/v1/calendar-feeds/tok-en_1.ics",
	webcal_url: "webcal://api.example.com/api/v1/calendar-feeds/tok-en_1.ics",
};

describe("feedFlags", () => {
	test("a bot with none of calendar, scheduling and memory has no feed", () => {
		expect(feedFlags({ capabilities: ["internet", "storage"] })).toBeNull();
	});

	test("each capability adds its layer, notes always follow", () => {
		expect(feedFlags({ capabilities: ["calendar"] })).toEqual([
			"include_events",
			"include_notes",
		]);
		expect(
			feedFlags({ capabilities: ["memory", "scheduling", "calendar"] }),
		).toEqual([
			"include_events",
			"include_tasks",
			"include_memories",
			"include_notes",
		]);
	});
});

describe("feedTargets", () => {
	const targets = Object.fromEntries(
		feedTargets(feed, "Secretary · Flow-Like").map((target) => [
			target.id,
			target.href,
		]),
	);

	test("Apple opens the webcal link as it is", () => {
		expect(targets.apple).toBe(feed.webcal_url);
	});

	test("Google takes the encoded webcal link as cid", () => {
		expect(targets.google).toBe(
			`https://calendar.google.com/calendar/r?cid=${encodeURIComponent(feed.webcal_url)}`,
		);
	});

	test("Outlook and Microsoft 365 take the encoded https link and name", () => {
		const query = `url=${encodeURIComponent(feed.url)}&name=${encodeURIComponent("Secretary · Flow-Like")}`;
		expect(targets.outlook).toBe(
			`https://outlook.live.com/calendar/0/addfromweb?${query}`,
		);
		expect(targets.microsoft).toBe(
			`https://outlook.office.com/calendar/0/addfromweb?${query}`,
		);
	});
});

test("a new feed starts with events and tasks only", () => {
	expect(FEED_DRAFT).toEqual({
		include_events: true,
		include_tasks: true,
		include_memories: false,
		include_notes: false,
	});
	expect(feedCalendarName("Secretary")).toBe("Secretary · Flow-Like");
});
