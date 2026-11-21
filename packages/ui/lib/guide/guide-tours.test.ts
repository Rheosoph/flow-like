import { describe, expect, test } from "bun:test";
import type { GuideEventName } from "./guide-events";
import {
	type GuideSignal,
	type IGuideRun,
	applyLocation,
	applySignal,
	currentStep,
	isRunFinished,
	startRun,
} from "./guide-model";
import { GUIDE_TOURS, guideTargetForNav } from "./guide-tours";

const event = (name: GuideEventName): GuideSignal => ({ kind: "event", name });

function walk(
	tour: (typeof GUIDE_TOURS)["use"],
	moves: readonly (GuideSignal | string)[],
): IGuideRun {
	let run = startRun(tour);
	for (const move of moves) {
		run =
			typeof move === "string"
				? applyLocation(tour, run, move)
				: applySignal(tour, run, move);
	}
	return run;
}

describe("the bots and apps tour", () => {
	const tour = GUIDE_TOURS.use;

	test("goes from the sidebar to an app without an approval", () => {
		const run = walk(tour, [
			"/bots",
			event("bots:chat-opened"),
			event("bots:message-sent"),
			event("bots:reply-finished"),
			event("bots:details-opened"),
			"/store/explore",
			event("store:app-opened"),
		]);
		expect(isRunFinished(tour, run)).toBe(true);
	});

	test("typing straight into the open chat passes the crew step", () => {
		const run = walk(tour, ["/bots", event("bots:message-sent")]);
		expect(currentStep(tour, run)?.id).toBe("watch");
	});

	test("an approval inserts its own step", () => {
		const run = walk(tour, [
			"/bots",
			event("bots:message-sent"),
			event("bots:approval-shown"),
		]);
		expect(currentStep(tour, run)?.id).toBe("approve");
		const answered = applySignal(tour, run, event("bots:approval-answered"));
		expect(currentStep(tour, answered)?.id).toBe("details");
	});
});

describe("the builder tour", () => {
	const tour = GUIDE_TOURS.build;

	test("goes from My Apps through a new flow to its first run", () => {
		const run = walk(tour, [
			"/library",
			event("library:create-opened"),
			event("library:app-created"),
			"/library/config",
			"/library/config/flows",
			event("flows:create-opened"),
			event("flows:board-created"),
			"/flow",
			event("board:node-added"),
			event("board:pins-connected"),
			event("board:run-started"),
			event("board:run-finished"),
		]);
		expect(currentStep(tour, run)?.id).toBe("logs");
	});

	test("landing on the board right after creating the app skips the flow setup", () => {
		const run = walk(tour, [
			"/library",
			event("library:create-opened"),
			event("library:app-created"),
			"/flow",
		]);
		expect(currentStep(tour, run)?.id).toBe("addEvent");
	});
});

test("sidebar destinations map to the tour's targets", () => {
	expect(guideTargetForNav("/bots")).toBe("nav-bots");
	expect(guideTargetForNav("/library")).toBe("nav-library");
	expect(guideTargetForNav("/store/explore")).toBe("nav-explore");
	expect(guideTargetForNav("/")).toBeUndefined();
});
