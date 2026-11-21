import { describe, expect, test } from "bun:test";
import {
	type IGuideTour,
	applyLocation,
	applySignal,
	currentStep,
	isRunFinished,
	routeMatches,
	skipStep,
	startRun,
	visibleProgress,
} from "./guide-model";

const TOUR: IGuideTour = {
	id: "use",
	flags: { "bots:approval-shown": "approval" },
	steps: [
		{ id: "bots", target: "nav-bots", completeOn: [{ route: "/bots" }] },
		{
			id: "send",
			target: "bot-composer",
			route: "/bots",
			completeOn: [{ event: "bots:message-sent" }],
		},
		{
			id: "watch",
			completeOn: [
				{ event: "bots:reply-finished" },
				{ event: "bots:approval-shown" },
			],
		},
		{
			id: "approve",
			requires: "approval",
			completeOn: [{ event: "bots:approval-answered" }],
		},
		{
			id: "details",
			target: "bot-pill",
			completeOn: [{ event: "bots:details-opened" }],
		},
	],
};

const at = (run: ReturnType<typeof startRun>) => currentStep(TOUR, run)?.id;

describe("routeMatches", () => {
	test("matches the route and its sub paths only", () => {
		expect(routeMatches("/bots", "/bots")).toBe(true);
		expect(routeMatches("/bots/abc", "/bots")).toBe(true);
		expect(routeMatches("/botsy", "/bots")).toBe(false);
		expect(routeMatches("/library", "/")).toBe(false);
		expect(routeMatches("/", "/")).toBe(true);
	});
});

describe("guide runs", () => {
	test("a step completes only on its own trigger", () => {
		let run = startRun(TOUR);
		run = applySignal(TOUR, run, { kind: "event", name: "bots:message-sent" });
		expect(at(run)).toBe("bots");
		run = applySignal(TOUR, run, { kind: "route", path: "/bots" });
		expect(at(run)).toBe("send");
	});

	test("an optional step is skipped when its flag never came up", () => {
		let run = applyLocation(TOUR, startRun(TOUR), "/bots");
		run = applySignal(TOUR, run, { kind: "event", name: "bots:message-sent" });
		run = applySignal(TOUR, run, {
			kind: "event",
			name: "bots:reply-finished",
		});
		expect(at(run)).toBe("details");
	});

	test("an optional step runs when its flag came up", () => {
		let run = applyLocation(TOUR, startRun(TOUR), "/bots");
		run = applySignal(TOUR, run, { kind: "event", name: "bots:message-sent" });
		run = applySignal(TOUR, run, {
			kind: "event",
			name: "bots:approval-shown",
		});
		expect(at(run)).toBe("approve");
		expect(visibleProgress(TOUR, run)).toEqual({ index: 4, total: 5 });
		run = applySignal(TOUR, run, {
			kind: "event",
			name: "bots:approval-answered",
		});
		expect(at(run)).toBe("details");
	});

	test("arriving on a route completes every step it satisfies", () => {
		const run = applyLocation(TOUR, startRun(TOUR), "/bots/assistant");
		expect(at(run)).toBe("send");
	});

	test("skipping and finishing", () => {
		let run = startRun(TOUR);
		for (let i = 0; i < 3; i += 1) run = skipStep(TOUR, run);
		expect(at(run)).toBe("details");
		expect(isRunFinished(TOUR, run)).toBe(false);
		run = applySignal(TOUR, run, {
			kind: "event",
			name: "bots:details-opened",
		});
		expect(isRunFinished(TOUR, run)).toBe(true);
		expect(skipStep(TOUR, run)).toBe(run);
	});

	test("progress counts only the steps the user sees", () => {
		const run = startRun(TOUR);
		expect(visibleProgress(TOUR, run)).toEqual({ index: 1, total: 4 });
	});
});
