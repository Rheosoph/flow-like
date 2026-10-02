import { describe, expect, test } from "bun:test";
import type { DeployOrder, DeployPhase } from "./deploy-plan";
import {
	type DeployRowState,
	type DeployRunEvent,
	type DeployRunState,
	type SharedPhase,
	activeTargets,
	createDeployRun,
	deployRunResult,
	reduceDeployRun,
} from "./deploy-run";

const PHASES: DeployPhase[] = ["upload", "install", "create", "start"];

function init(
	order: DeployOrder = "one",
	stopOnFail = true,
	shared: SharedPhase | null = null,
): DeployRunState {
	return createDeployRun({
		id: "run-1",
		order,
		stopOnFail,
		shared,
		rows: ["a", "b", "c"].map((target) => ({
			target,
			deviceId: `device-${target}`,
			serviceId: "crm-webhook",
			phases: PHASES,
		})),
	});
}

let clock = 0;
const at = () => ++clock;

const start = (): DeployRunEvent => ({ type: "start", at: at() });
const done = (target: string): DeployRunEvent => ({
	type: "done",
	target,
	at: at(),
});
const fail = (
	target: string,
	phase: DeployPhase = "install",
	rolledBack?: boolean,
): DeployRunEvent => ({
	type: "fail",
	target,
	at: at(),
	error: {
		phase,
		code: "checksum",
		detail: "2 files didn't match their checksums",
		...(rolledBack ? { rolledBack } : {}),
	},
});
const act = (
	type: "retry" | "skip" | "unblock",
	target: string,
): DeployRunEvent => ({ type, target, at: at() });
const all = (type: "continue" | "stop"): DeployRunEvent => ({ type, at: at() });

function play(state: DeployRunState, events: DeployRunEvent[]): DeployRunState {
	return events.reduce(reduceDeployRun, state);
}

function rows(state: DeployRunState): string {
	return state.rows.map((row) => `${row.target}:${row.state}`).join(" ");
}

describe("APP §7.8 row states", () => {
	const table: [DeployRowState, DeployRunState, string][] = [
		["waiting", play(init(), [start()]), "b"],
		["active", play(init(), [start()]), "a"],
		["done", play(init(), [start(), done("a")]), "a"],
		["failed", play(init(), [start(), fail("a")]), "a"],
		[
			"blocked",
			play(init(), [
				start(),
				{ type: "block", target: "a", at: at(), reason: "locked" },
			]),
			"a",
		],
		["skipped", play(init(), [start(), fail("a"), act("skip", "a")]), "a"],
		["held", play(init(), [start(), fail("a")]), "b"],
		["not_started", play(init(), [start(), fail("a"), all("stop")]), "c"],
	];
	for (const [state, run, target] of table)
		test(state, () => {
			expect(run.rows.find((row) => row.target === target)?.state).toBe(state);
		});

	test("an update that switched over and was restored keeps its rollback flag", () => {
		const run = play(init(), [start(), fail("a", "switch", true)]);
		expect(run.rows[0]).toMatchObject({
			state: "failed",
			phase: 0,
			error: { phase: "switch", rolledBack: true },
		});
		expect(deployRunResult(play(run, [all("stop")]))?.failed[0]).toMatchObject({
			target: "a",
			rolledBack: true,
		});
	});
});

describe("one device at a time, stop if a device fails", () => {
	const failedB = () => play(init(), [start(), done("a"), fail("b")]);

	test("a failure holds the rest until you continue or stop", () => {
		const held = failedB();
		expect([held.status, held.holdBy, rows(held)]).toEqual([
			"held",
			"b",
			"a:done b:failed c:held",
		]);
		expect(held.rows[1]).toMatchObject({
			phase: 1,
			error: { phase: "install" },
		});
		const continued = play(held, [all("continue")]);
		expect([continued.status, rows(continued)]).toEqual([
			"running",
			"a:done b:failed c:active",
		]);
		const ended = play(continued, [done("c")]);
		expect(deployRunResult(ended)).toMatchObject({
			outcome: "partial",
			done: ["a", "c"],
			failed: [{ target: "b", phase: "install", code: "checksum" }],
			skipped: [],
			notStarted: [],
		});
	});

	test("stop here leaves the rest not started and ends the run", () => {
		const stopped = play(failedB(), [all("stop")]);
		expect([stopped.status, stopped.stopped, rows(stopped)]).toEqual([
			"finished",
			true,
			"a:done b:failed c:not_started",
		]);
		expect(deployRunResult(stopped)).toMatchObject({
			outcome: "partial",
			notStarted: ["c"],
		});
	});

	test("retry resumes the failed device from its failed phase, then the rest", () => {
		const retried = play(failedB(), [act("retry", "b")]);
		expect([retried.status, retried.holdBy, rows(retried)]).toEqual([
			"running",
			null,
			"a:done b:active c:waiting",
		]);
		expect(retried.rows[1]).toMatchObject({ phase: 1, error: undefined });
		const next = play(retried, [done("b")]);
		expect(rows(next)).toBe("a:done b:done c:active");
		expect(deployRunResult(play(next, [done("c")]))?.outcome).toBe("all");
	});

	test("retrying one of two failed devices never strands the held rest", () => {
		const twoFailed = play(init(), [
			start(),
			fail("a"),
			all("continue"),
			fail("b"),
		]);
		expect([twoFailed.status, twoFailed.holdBy, rows(twoFailed)]).toEqual([
			"held",
			"b",
			"a:failed b:failed c:held",
		]);
		const retrying = play(twoFailed, [act("retry", "a")]);
		expect([retrying.status, rows(retrying)]).toEqual([
			"running",
			"a:active b:failed c:held",
		]);
		const waiting = play(retrying, [done("a")]);
		expect([waiting.status, waiting.holdBy, rows(waiting)]).toEqual([
			"held",
			"b",
			"a:done b:failed c:held",
		]);
		const continued = play(waiting, [all("continue")]);
		expect(rows(continued)).toBe("a:done b:failed c:active");
		expect(deployRunResult(play(continued, [done("c")]))).toMatchObject({
			outcome: "partial",
			done: ["a", "c"],
		});
		const early = play(retrying, [all("continue"), done("a")]);
		expect(rows(early)).toBe("a:done b:failed c:active");
	});

	test("skip moves on to the next device", () => {
		const skipped = play(failedB(), [act("skip", "b")]);
		expect([skipped.status, rows(skipped)]).toEqual([
			"running",
			"a:done b:skipped c:active",
		]);
		expect(deployRunResult(play(skipped, [done("c")]))).toMatchObject({
			outcome: "partial",
			skipped: ["b"],
		});
	});

	test("a finished partial run can retry its failed device", () => {
		const finished = play(failedB(), [all("stop")]);
		const retried = play(finished, [act("retry", "b")]);
		expect([retried.status, retried.finishedAt, rows(retried)]).toEqual([
			"running",
			undefined,
			"a:done b:active c:not_started",
		]);
		expect(deployRunResult(play(retried, [done("b")]))).toMatchObject({
			outcome: "partial",
			done: ["a", "b"],
			notStarted: ["c"],
		});
	});
});

describe("orders", () => {
	test("all at once starts every device; a failure holds nothing that already runs", () => {
		const started = play(init("all"), [start()]);
		expect(activeTargets(started)).toEqual(["a", "b", "c"]);
		const failed = play(started, [fail("a")]);
		expect([failed.status, rows(failed)]).toEqual([
			"running",
			"a:failed b:active c:active",
		]);
		const ended = play(failed, [done("b"), done("c")]);
		expect([ended.status, rows(ended)]).toEqual([
			"finished",
			"a:failed b:done c:done",
		]);
	});

	test("a retried device that fails again holds the rest only once nothing else runs", () => {
		const both = play(init(), [
			start(),
			fail("a"),
			all("continue"),
			act("retry", "a"),
		]);
		expect(rows(both)).toBe("a:active b:active c:waiting");
		const again = play(both, [fail("a")]);
		expect([again.status, again.holdBy, rows(again)]).toEqual([
			"running",
			"a",
			"a:failed b:active c:held",
		]);
		const held = play(again, [done("b")]);
		expect([held.status, rows(held)]).toEqual([
			"held",
			"a:failed b:done c:held",
		]);
	});

	test("a failure in a phase the row doesn't list keeps the phase it had reached", () => {
		const run = play(init(), [
			start(),
			{ type: "phase", target: "a", phase: "create", at: at() },
			fail("a", "switch"),
		]);
		expect(run.rows[0]).toMatchObject({ state: "failed", phase: 2 });
	});

	test("first device, then the rest", () => {
		const first = play(init("first"), [start()]);
		expect(activeTargets(first)).toEqual(["a"]);
		expect(activeTargets(play(first, [done("a")]))).toEqual(["b", "c"]);
		const failedFirst = play(init("first", false), [start(), fail("a")]);
		expect([failedFirst.status, activeTargets(failedFirst)]).toEqual([
			"running",
			["b", "c"],
		]);
	});

	test("without stop-on-fail a failure moves straight on", () => {
		const run = play(init("one", false), [
			start(),
			fail("a"),
			done("b"),
			done("c"),
		]);
		expect(deployRunResult(run)).toMatchObject({
			outcome: "partial",
			done: ["b", "c"],
		});
	});

	test("nothing deployed", () => {
		const run = play(init("one", false), [
			start(),
			fail("a", "upload"),
			fail("b", "upload"),
			fail("c", "upload"),
		]);
		expect(deployRunResult(run)?.outcome).toBe("none");
	});
});

describe("blocked rows, phases and the shared phase", () => {
	test("a blocked device waits for unlock and holds its turn", () => {
		const blocked = play(init(), [
			start(),
			{ type: "block", target: "a", at: at(), reason: "locked" },
		]);
		expect([rows(blocked), blocked.rows[0].blocked]).toEqual([
			"a:blocked b:waiting c:waiting",
			"locked",
		]);
		expect(rows(play(blocked, [act("unblock", "a")]))).toBe(
			"a:active b:waiting c:waiting",
		);
		expect(rows(play(blocked, [act("skip", "a")]))).toBe(
			"a:skipped b:active c:waiting",
		);
	});

	test("phase events move the active row and carry progress", () => {
		const run = play(init(), [
			start(),
			{
				type: "phase",
				target: "a",
				phase: "upload",
				at: at(),
				progress: { done: 14, total: 26 },
			},
			{ type: "phase", target: "a", phase: "create", at: at() },
		]);
		expect(run.rows[0]).toMatchObject({ phase: 2, progress: undefined });
		const ignored = play(run, [
			{ type: "phase", target: "b", phase: "install", at: at() },
			{ type: "phase", target: "a", phase: "switch", at: at() },
		]);
		expect(ignored.rows.map((row) => row.phase)).toEqual([2, 0, 0]);
		expect(play(run, [done("a")]).rows[0].phase).toBe(PHASES.length);
	});

	test("the shared phase runs once before any device", () => {
		const started = play(init("one", true, "prepare"), [start()]);
		expect([started.shared?.state, activeTargets(started)]).toEqual([
			"active",
			[],
		]);
		const ready = play(started, [{ type: "shared_done", at: at() }]);
		expect(activeTargets(ready)).toEqual(["a"]);
		const failed = play(started, [
			{
				type: "shared_fail",
				at: at(),
				error: { phase: "upload", code: "saved_secret" },
			},
		]);
		expect([failed.status, rows(failed)]).toEqual([
			"finished",
			"a:not_started b:not_started c:not_started",
		]);
		expect(deployRunResult(failed)).toMatchObject({
			outcome: "none",
			sharedFailure: { code: "saved_secret" },
		});
	});

	test("events that don't apply leave the state unchanged", () => {
		const idle = init();
		expect(reduceDeployRun(idle, done("a"))).toBe(idle);
		expect(reduceDeployRun(idle, all("continue"))).toBe(idle);
		const running = play(idle, [start()]);
		expect(reduceDeployRun(running, start())).toBe(running);
		expect(reduceDeployRun(running, act("retry", "a"))).toBe(running);
		expect(reduceDeployRun(running, fail("b"))).toBe(running);
		expect(reduceDeployRun(running, act("skip", "zzz"))).toBe(running);
		expect(reduceDeployRun(running, { type: "shared_done", at: at() })).toBe(
			running,
		);
		const finished = play(running, [done("a"), done("b"), done("c")]);
		expect(finished.status).toBe("finished");
		expect(reduceDeployRun(finished, all("stop"))).toBe(finished);
		expect(deployRunResult(running)).toBeNull();
	});
});
