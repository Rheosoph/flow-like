import { describe, expect, test } from "bun:test";
import { FORM_LIMITS } from "../contracts";
import { type FixtureName, fixture, fixtureClock } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";
import { outputWith, sessionDriver } from "./reduce-driver";

const from = (name: FixtureName) =>
	sessionDriver(fixture(name), fixtureClock(name));

describe("the stage and the compared run (spec 2.0, M5)", () => {
	test("a picked run is compared with the rail and its failure counts as seen", () => {
		const driver = from("series-failed");
		const failed = driver.state.runs.find((run) => run.n === 18);
		expect(failed?.unseenFailure).toBe(true);
		driver.command({
			type: "selectRun",
			runId: failed?.id ?? null,
			how: "menu",
		});
		const after = driver.state.runs.find((run) => run.n === 18);
		expect(after?.unseenFailure).toBe(false);
		expect(driver.state.view.selectedRunId).toBe(failed?.id ?? "");
		expect(driver.state.rail.comparedRunId).toBe(failed?.id ?? "");
		expect(driver.state.view.stageFollowsNewest).toBe(false);
	});

	test("the newest run on opening is context, not a comparison", () => {
		const driver = from("reopen");
		driver.command({ type: "selectRun", runId: "run-14", how: "auto" });
		expect(driver.state.rail.comparedRunId).toBeNull();
		driver.command({ type: "selectRun", runId: "run-14", how: "tab" });
		expect(driver.state.rail.comparedRunId).toBe("run-14");
		expect(driver.state.view.stageFollowsNewest).toBe(true);
	});

	test("markSeen; pin and unpin; an unknown run is ignored", () => {
		const driver = from("series-failed");
		const failed = driver.state.runs.find((run) => run.n === 18);
		driver.command({ type: "markSeen", runId: failed?.id ?? "" });
		expect(driver.state.runs.find((run) => run.n === 18)?.unseenFailure).toBe(
			false,
		);
		driver.command({ type: "pinRun", runId: "run-15" });
		expect(driver.state.view.pinnedRunId).toBe("run-15");
		driver.command({ type: "pinRun", runId: null });
		expect(driver.state.view.pinnedRunId).toBeNull();
		const before = driver.state;
		driver.command({ type: "selectRun", runId: "nope", how: "tab" });
		expect(driver.state).toBe(before);
	});
});

describe("panes on a phone", () => {
	test("a run that ends while the Inputs pane shows marks the Output segment; showing Output clears it", () => {
		const driver = from("running");
		driver.command({ type: "setLayout", layout: PHONE_LAYOUT });
		driver.command({ type: "setPane", pane: "inputs" });
		driver.settle("run-14", { kind: "succeeded" }, outputWith("Done."));
		expect(driver.state.view.outputUnseen).toBe(true);
		driver.command({ type: "setPane", pane: "output" });
		expect(driver.state.view.outputUnseen).toBe(false);
	});

	test("a Runs row picked on a phone shows the Output pane", () => {
		const driver = from("runs");
		driver.command({ type: "setLayout", layout: PHONE_LAYOUT });
		driver.command({ type: "selectRun", runId: "run-12", how: "list" });
		expect(driver.state.view.pane).toBe("output");
	});

	test("the same layout reported again changes nothing", () => {
		const driver = from("idle");
		const before = driver.state;
		driver.command({ type: "setLayout", layout: DESKTOP_LAYOUT });
		expect(driver.state).toBe(before);
	});
});

describe("lists under a field (spec M3, M6)", () => {
	test("recent values open on the first row; ↑ / ↓ stay inside the list", () => {
		const driver = from("reopen");
		driver.command({ type: "openList", kind: "recent", key: "vendor_name" });
		expect(driver.state.view.list).toEqual({
			kind: "recent",
			key: "vendor_name",
			active: 0,
		});
		driver.command({ type: "moveList", delta: 1 });
		expect(driver.state.view.list?.active).toBe(1);
		driver.command({ type: "moveList", delta: 1 });
		expect(driver.state.view.list?.active).toBe(1);
		driver.command({ type: "moveList", delta: -5 });
		expect(driver.state.view.list?.active).toBe(0);
		driver.command({ type: "closeList" });
		expect(driver.state.view.list).toBeNull();
	});

	test("Delete on the active recent value keeps the active row inside the list; the last one closes it", () => {
		const driver = from("reopen");
		driver.command({ type: "openList", kind: "recent", key: "vendor_name" });
		driver.command({ type: "moveList", delta: 1 });
		driver.command({
			type: "forgetRecent",
			name: "vendor_name",
			value: "Alpenfracht AG",
		});
		expect(driver.state.view.list?.active).toBe(0);
		driver.command({
			type: "forgetRecent",
			name: "vendor_name",
			value: "Nordwind Logistik GmbH",
		});
		expect(driver.state.view.list).toBeNull();
	});

	test("next files open with no row active", () => {
		const driver = from("series");
		driver.command({
			type: "openList",
			kind: "nextFiles",
			key: "invoice_file",
		});
		expect(driver.state.view.list?.active).toBe(-1);
		driver.command({ type: "moveList", delta: 1 });
		expect(driver.state.view.list?.active).toBe(0);
		driver.command({ type: "moveList", delta: 10 });
		expect(driver.state.view.list?.active).toBe(4);
	});
});

describe("focus requests and the dock message", () => {
	test("focusHandled clears only the request it names", () => {
		const driver = from("invalid");
		const seq = driver.state.view.focus?.seq ?? -1;
		driver.command({ type: "focusHandled", seq: seq + 1 });
		expect(driver.state.view.focus?.seq).toBe(seq);
		driver.command({ type: "focusHandled", seq });
		expect(driver.state.view.focus).toBeNull();
	});

	test("a key press in the rail ends a start message; other messages wait for their time", () => {
		const driver = from("series");
		expect(driver.state.view.message?.message.kind).toBe("start");
		driver.command({ type: "railKey" });
		expect(driver.state.view.message).toBeNull();
		driver.command({ type: "enter", fromKey: "vendor_name" });
		expect(driver.state.view.message).toBeNull();
	});

	test("a plain message goes after 4 s, by its timer or on the next step after its time", () => {
		const driver = from("done");
		driver.command({ type: "enter", fromKey: "vendor_name" });
		const entry = driver.state.view.message;
		expect(entry?.expiresAt).toBe(driver.now + FORM_LIMITS.messageMs);
		expect(driver.last).toContainEqual({
			type: "expireMessage",
			seq: entry?.seq ?? 0,
			afterMs: FORM_LIMITS.messageMs,
		});
		driver.input({ type: "messageExpired", seq: (entry?.seq ?? 0) + 1 });
		expect(driver.state.view.message).toBe(entry);
		driver
			.advance(FORM_LIMITS.messageMs)
			.command({ type: "setRailTab", tab: "runs" });
		expect(driver.state.view.message).toBeNull();
	});

	test("dismissing a message keeps Undo (⌘Z still works)", () => {
		const driver = from("idle");
		driver.command({ type: "setValue", key: "max_pages", value: "60" });
		driver.command({ type: "resetField", key: "max_pages" });
		driver.command({ type: "dismissMessage" });
		expect(driver.state.view.message).toBeNull();
		expect(driver.state.undo?.kind).toBe("fieldReset");
	});

	test("overlays open and close; the leave dialog is an overlay", () => {
		const driver = from("series");
		driver.command({
			type: "openOverlay",
			overlay: { id: "leave", route: "/", replace: false },
		});
		expect(driver.state.view.overlay).toEqual({
			id: "leave",
			route: "/",
			replace: false,
		});
		driver.command({ type: "closeOverlay" });
		expect(driver.state.view.overlay).toBeNull();
		expect(driver.state.runs.find((run) => run.n === 18)?.status).toBe(
			"queued",
		);
	});
});

describe("answers to a run's question", () => {
	const busyRun = (driver: ReturnType<typeof from>) => {
		const run = driver.state.runs.find((item) =>
			["asking", "running", "streaming"].includes(item.status),
		);
		if (!run) throw new Error("the fixture has no run that still goes");
		return run;
	};

	test("an answer goes to the runtime as it was given", () => {
		const driver = from("running");
		const run = busyRun(driver);
		driver.command({
			type: "respondInteraction",
			runId: run.id,
			interactionId: "q-1",
			value: "yes",
		});
		expect(driver.last).toEqual([
			{
				type: "respondInteraction",
				runId: run.id,
				interactionId: "q-1",
				value: "yes",
			},
		]);
	});

	test("a failed answer is said in the dock until the next try, without a timer", () => {
		const driver = from("running");
		const run = busyRun(driver);
		driver.input({
			type: "interactionFailed",
			runId: run.id,
			interactionId: "q-1",
			error: "channel closed",
		});
		const entry = driver.state.view.message;
		expect(entry?.message).toEqual({
			kind: "answerFailed",
			n: run.n,
			runId: run.id,
		});
		expect(entry?.expiresAt).toBeNull();
		expect(entry?.undo).toBe(false);
		expect(
			driver.last.filter((effect) => effect.type === "expireMessage"),
		).toEqual([]);
		driver.advance(FORM_LIMITS.messageMs * 3);
		driver.command({ type: "focusHandled", seq: -1 });
		expect(driver.state.view.message).toBe(entry);
		driver.command({
			type: "respondInteraction",
			runId: run.id,
			interactionId: "q-1",
			value: "yes",
		});
		expect(driver.state.view.message).toBeNull();
	});

	test("the message goes when its run ends: nothing is left to answer", () => {
		const driver = from("running");
		const run = busyRun(driver);
		driver.input({
			type: "interactionFailed",
			runId: run.id,
			interactionId: "q-1",
			error: "channel closed",
		});
		expect(driver.state.view.message?.message.kind).toBe("answerFailed");
		driver.settle(run.id, { kind: "succeeded" }, outputWith("Booked", 3));
		expect(driver.state.runs.find((item) => item.id === run.id)?.status).toBe(
			"done",
		);
		expect(driver.state.view.message).toBeNull();
	});

	test("a run that ended or is unknown gets no message", () => {
		const driver = from("done");
		const ended = driver.state.runs[0];
		driver.input({
			type: "interactionFailed",
			runId: ended.id,
			interactionId: "q-1",
			error: "ended",
		});
		driver.input({
			type: "interactionFailed",
			runId: "run-gone",
			interactionId: "q-1",
			error: "unknown",
		});
		expect(driver.state.view.message).toBeNull();
	});
});
