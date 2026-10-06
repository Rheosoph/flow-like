import { describe, expect, test } from "bun:test";
import { routeLabelOf, routeLabelsOf } from "./route-labels";

describe("routeLabelOf", () => {
	test("the routed event's name wins", () => {
		expect(routeLabelOf("/support", "Support chat", "Home")).toBe(
			"Support chat",
		);
		expect(routeLabelOf("/", "Start page", "Home")).toBe("Start page");
	});

	test('"/" is Home', () => {
		expect(routeLabelOf("/", undefined, "Home")).toBe("Home");
		expect(routeLabelOf("/", undefined, "Startseite")).toBe("Startseite");
	});

	test("a path reads as words, first letter up", () => {
		expect(routeLabelOf("/review-queue", undefined, "Home")).toBe(
			"Review queue",
		);
		expect(routeLabelOf("/support/open-tickets", undefined, "Home")).toBe(
			"Support / open tickets",
		);
	});
});

describe("routeLabelsOf", () => {
	const events = [
		{ id: "e1", name: "Review queue" },
		{ id: "e2", name: "" },
		{ id: "e3", name: "Support chat" },
	];

	test("names come from the event each route maps to; the rest from the path", () => {
		const labels = routeLabelsOf({
			routes: ["/", "/review", "/help-desk", "/chat"],
			mappings: [
				{ path: "/review", eventId: "e1" },
				{ path: "/help-desk", eventId: "e2" },
				{ path: "/chat", eventId: "e3" },
			],
			events,
			homeLabel: "Home",
		});
		expect(labels).toEqual({
			"/": "Home",
			"/review": "Review queue",
			"/help-desk": "Help desk",
			"/chat": "Support chat",
		});
	});

	test("a stored route without its leading slash still finds its event", () => {
		const labels = routeLabelsOf({
			routes: ["/review"],
			mappings: [{ path: "review", eventId: "e1" }],
			events,
			homeLabel: "Home",
		});
		expect(labels["/review"]).toBe("Review queue");
	});

	test("without routes data every route reads from its path", () => {
		const labels = routeLabelsOf({
			routes: ["/", "/review-queue"],
			mappings: [],
			events: [],
			homeLabel: "Home",
		});
		expect(labels).toEqual({ "/": "Home", "/review-queue": "Review queue" });
	});
});
