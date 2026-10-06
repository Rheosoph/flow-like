import { describe, expect, test } from "bun:test";
import {
	FORM_LIMITS,
	type FileSlot,
	type FormModel,
	type HostCapabilities,
	type SessionEffect,
} from "../contracts";
import {
	type FixtureName,
	fixture,
	fixtureClock,
	fixtureForm,
} from "../testing/fixtures";
import { PHONE_LAYOUT } from "../testing/layouts";
import { resolveHostCapabilities } from "./host";
import { distinctEffects } from "./reduce";
import {
	type SessionDriver,
	fileNames,
	focusedKey,
	pickedFiles,
	sessionDriver,
} from "./reduce-driver";
import { initialSessionState } from "./state";

const from = (name: FixtureName) =>
	sessionDriver(fixture(name), fixtureClock(name));

const ofType = <T extends SessionEffect["type"]>(
	effects: readonly SessionEffect[],
	type: T,
) =>
	effects.filter(
		(effect): effect is Extract<SessionEffect, { type: T }> =>
			effect.type === type,
	);

/** Return request with an order number typed over and a receipt picked and sent. */
function fillSmall(
	driver: SessionDriver,
	order = "48213-7",
	receipt = "r1.jpg",
) {
	driver.command({
		type: "setValue",
		key: "order",
		value: order,
		how: "replace",
	});
	driver.command({
		type: "pickFiles",
		name: "receipt",
		files: pickedFiles([{ name: receipt, size: 400_000 }]),
		mode: "replace",
	});
	driver.sendUploads();
	return driver;
}

describe("a press with problems", () => {
	test("messages show, the dock counts them, focus goes to the first; nothing runs", () => {
		const driver = from("idle");
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		const { rail } = driver.state;
		expect(Object.keys(rail.problems)).toEqual([
			"invoice_file",
			"vendor_name",
			"invoice_date",
		]);
		expect(rail.pressed).toBe(true);
		expect(focusedKey(driver.state)).toBe("invoice_file");
		expect(driver.state.runs).toEqual([]);
		expect(ofType(driver.last, "persistRun")).toEqual([]);
	});

	test("an unreadable typed date is one of them", () => {
		const driver = from("done");
		driver.command({ type: "setText", key: "invoice_date", text: "31" });
		driver.command({ type: "run", leaveAsIs: false, from: "chord" });
		expect(driver.state.rail.problems).toEqual({
			invoice_date: { code: "date" },
		});
		expect(driver.state.runs[0].n).toBe(14);
	});
});

describe("the press", () => {
	test("the run takes its copy, joins the queue and starts; the number and the run are saved", () => {
		const driver = fillSmall(from("small"));
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		const [run] = driver.state.runs;
		expect(run.n).toBe(1);
		expect(run.origin).toBe("session");
		expect(run.status).toBe("starting");
		expect(run.copy.values.order).toBe("48213-7");
		expect(fileNames(run.copy.values.receipt)).toBe("r1.jpg");
		expect(run.copy.replaced).toEqual(["order"]);
		expect(ofType(driver.last, "dispatchRun")).toEqual([
			{ type: "dispatchRun", runId: run.id },
		]);
		const saved = ofType(driver.last, "persistRun");
		expect(saved).toHaveLength(1);
		expect(saved[0].record.status).toBe("running");
		expect(ofType(driver.last, "persistPrefs")[0].prefs.nextRunNumber).toBe(2);
		expect(driver.state.rail.lastPressAt).toBe(driver.now);
		expect(distinctEffects(driver.last)).toHaveLength(driver.last.length);
	});

	test("nothing per run: inputs survive the run and the cursor stays", () => {
		const driver = fillSmall(from("small"));
		const focus = driver.state.view.focus;
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		expect(driver.state.rail.values.order).toBe("48213-7");
		expect(driver.state.view.focus).toBe(focus);
		expect(driver.state.view.message).toBeNull();
	});

	test("A's 700 ms rule: the same inputs pressed again within 700 ms start nothing", () => {
		const driver = fillSmall(from("small"));
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		driver.advance(FORM_LIMITS.doublePressMs - 1);
		driver.command({ type: "run", leaveAsIs: false, from: "enter" });
		expect(driver.state.runs).toHaveLength(1);
		driver.advance(1);
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		expect(driver.state.runs).toHaveLength(2);
	});

	test("per-run fields go back to their starting value; the copy keeps what was typed", () => {
		const driver = from("small");
		driver.command({ type: "setPerRun", name: "order", on: true });
		fillSmall(driver);
		driver.command({ type: "run", leaveAsIs: false, from: "chord" });
		const [run] = driver.state.runs;
		expect(run.copy.values.order).toBe("48213-7");
		expect(run.copy.perRun).toEqual(["order"]);
		expect(driver.state.rail.values.order).toBe("");
		expect(fileNames(driver.state.rail.values.receipt)).toBe("r1.jpg");
		expect(driver.state.view.focus?.target).toEqual({
			kind: "field",
			key: "order",
			select: true,
			scrollOnly: false,
		});
		expect(driver.state.view.message?.message).toEqual({
			kind: "start",
			start: {
				n: 1,
				runId: run.id,
				state: "started",
				files: 0,
				pairs: ["48213-7"],
				lastFile: false,
				leftAsIs: false,
			},
		});
	});

	test("⇧⌘↵ runs and leaves the inputs as they are this time", () => {
		const driver = from("small");
		driver.command({ type: "setPerRun", name: "order", on: true });
		fillSmall(driver);
		const focus = driver.state.view.focus;
		driver.command({ type: "run", leaveAsIs: true, from: "chord" });
		expect(driver.state.rail.values.order).toBe("48213-7");
		expect(driver.state.runs[0].copy.leftAsIs).toBe(true);
		const message = driver.state.view.message?.message;
		expect(message?.kind === "start" && message.start.leftAsIs).toBe(true);
		expect(driver.state.view.focus).toBe(focus);
	});

	test("a file still sending: the run waits in Sending, then starts once it is sent", () => {
		const driver = from("small");
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r.jpg", size: 10 }]),
			mode: "replace",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		const [run] = driver.state.runs;
		expect(run.status).toBe("sending");
		expect(run.pendingSlotIds).toEqual(["pick-r.jpg"]);
		const message = driver.state.view.message?.message;
		expect(message?.kind === "start" && message.start.state).toBe("sending");
		expect(message?.kind === "start" && message.start.files).toBe(1);
		expect(driver.state.view.selectedRunId).toBeNull();
		driver.sendUploads();
		expect(driver.state.runs[0].status).toBe("starting");
		expect(driver.state.runs[0].pendingSlotIds).toEqual([]);
		expect(ofType(driver.last, "dispatchRun")).toHaveLength(1);
		expect(driver.state.view.message).toBeNull();
	});
});

describe("the start message follows its run", () => {
	/** Order per run, a receipt still sending: "Run 1 starts when its file is sent: 1." `cap` 0: no place is free. */
	function sendingRun(cap?: number) {
		const small = fixture("small");
		const driver = sessionDriver(
			cap === undefined ? small : { ...small, queue: { ...small.queue, cap } },
			fixtureClock("small"),
		);
		driver.command({ type: "setPerRun", name: "order", on: true });
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r.jpg", size: 10 }]),
			mode: "replace",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		const entry = driver.state.view.message;
		expect(entry?.message).toMatchObject({
			kind: "start",
			start: { n: 1, state: "sending", files: 1, pairs: ["1"] },
		});
		return { driver, entry, run: driver.state.runs[0] };
	}

	test("once its file is sent the same message says the run started, and keeps its time", () => {
		const { driver, entry } = sendingRun();
		driver.advance(1000);
		driver.sendUploads();
		expect(driver.state.runs[0].status).toBe("starting");
		const now = driver.state.view.message;
		expect(now?.message).toMatchObject({
			kind: "start",
			start: { n: 1, state: "started", files: 1, pairs: ["1"] },
		});
		expect(now?.seq).toBe(entry?.seq);
		expect(now?.expiresAt).toBe(entry?.expiresAt);
	});

	test("sent while no place is free, it says the run is queued", () => {
		const { driver } = sendingRun(0);
		driver.sendUploads();
		expect(driver.state.runs[0].status).toBe("queued");
		expect(driver.state.view.message?.message).toMatchObject({
			kind: "start",
			start: { state: "queued", pairs: ["1"] },
		});
	});

	test("taken out before it began, the run has no start message", () => {
		const { driver, run } = sendingRun();
		driver.command({ type: "removeFromQueue", runId: run.id });
		expect(driver.state.runs[0].status).toBe("notStarted");
		expect(driver.state.view.message).toBeNull();
	});

	test("a start message that went does not come back when its run starts", () => {
		const { driver } = sendingRun();
		driver.command({ type: "dismissMessage" });
		driver.sendUploads();
		expect(driver.state.runs[0].status).toBe("starting");
		expect(driver.state.view.message).toBeNull();
	});
});

describe("↵ and the newest run (spec M2)", () => {
	test("a run taken out before it began is no reason to refuse ↵", () => {
		const driver = from("small");
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r.jpg", size: 10 }]),
			mode: "replace",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		const [first] = driver.state.runs;
		driver.command({ type: "removeFromQueue", runId: first.id });
		driver.sendUploads();
		driver.advance(FORM_LIMITS.doublePressMs);
		driver.command({ type: "enter", fromKey: "order" });
		expect(driver.state.runs).toHaveLength(2);
		expect(driver.state.view.message?.message.kind).not.toBe("sameInputs");
	});

	test("the same inputs as a run that went: ↵ refuses and names it", () => {
		const driver = fillSmall(from("small"));
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		driver.advance(FORM_LIMITS.doublePressMs);
		driver.command({ type: "enter", fromKey: "order" });
		expect(driver.state.runs).toHaveLength(1);
		expect(driver.state.view.message?.message).toEqual({
			kind: "sameInputs",
			n: 1,
		});
	});
});

describe("a form without fields (spec M5)", () => {
	test("Run starts at once; the cursor moves to the strip's Run again", () => {
		const driver = from("none");
		driver.command({ type: "run", leaveAsIs: false, from: "hero" });
		expect(driver.state.runs[0].status).toBe("starting");
		expect(driver.state.view.focus?.target).toEqual({ kind: "runAgain" });
		expect(driver.state.view.selectedRunId).toBe(driver.state.runs[0].id);
	});

	test("at its cap a press is refused and announced, never queued", () => {
		const driver = from("none");
		for (let press = 0; press < 3; press += 1)
			driver
				.command({ type: "run", leaveAsIs: false, from: "strip" })
				.advance(1000);
		expect(driver.state.runs.map((run) => run.status)).toEqual([
			"starting",
			"starting",
			"starting",
		]);
		driver.command({ type: "run", leaveAsIs: false, from: "strip" });
		expect(driver.state.runs).toHaveLength(3);
		expect(driver.state.announcement).toEqual({
			seq: driver.state.announcement?.seq ?? 0,
			kind: "capReached",
			running: 3,
		});
	});

	test("a double click starts one run", () => {
		const driver = from("none");
		driver.command({ type: "run", leaveAsIs: false, from: "hero" });
		driver
			.advance(200)
			.command({ type: "run", leaveAsIs: false, from: "strip" });
		expect(driver.state.runs).toHaveLength(1);
	});
});

describe("the phone (spec M1)", () => {
	test("while next files wait Run keeps the Inputs pane; the next field scrolls into view without focus", () => {
		const driver = from("series");
		driver.command({ type: "setLayout", layout: PHONE_LAYOUT });
		driver.command({
			type: "setValue",
			key: "invoice_date",
			value: "2026-09-23",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "phone" });
		expect(driver.state.view.pane).toBe("inputs");
		expect(driver.state.view.focus?.target).toEqual({
			kind: "field",
			key: "invoice_date",
			select: false,
			scrollOnly: true,
		});
	});

	test("otherwise Run switches to the Output pane; with problems it stays on Inputs", () => {
		const driver = fillSmall(from("small"));
		driver.command({ type: "setLayout", layout: PHONE_LAYOUT });
		driver.command({ type: "setPane", pane: "output" });
		driver.command({ type: "setValue", key: "order", value: "" });
		driver.command({ type: "run", leaveAsIs: false, from: "phone" });
		expect(driver.state.view.pane).toBe("inputs");
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({ type: "run", leaveAsIs: false, from: "phone" });
		expect(driver.state.view.pane).toBe("output");
	});
});

describe("inline hosts", () => {
	/** Return request on a device page, with a legacy (URL) receipt list. */
	function serviceForm(): FormModel {
		const host: HostCapabilities = resolveHostCapabilities({
			kind: "service",
			presentation: "page",
			helperState: { fileToUrl: async () => "" },
			hasToolbar: false,
			signedIn: false,
			memoryScope: null,
		});
		const form = fixtureForm("small", host);
		const fields = form.fields.map((field) =>
			field.name === "receipt"
				? {
						...field,
						kind: "files" as const,
						valueType: "Array",
						fileMode: "url" as const,
					}
				: field,
		);
		return { ...form, fields };
	}

	test("files are sent at once (no uploads); together too large for one run, the press is refused", () => {
		const form = serviceForm();
		const clock = fixtureClock("small");
		const driver = sessionDriver(initialSessionState(form, clock), clock);
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([
				{ name: "a.pdf", size: 3_000_000 },
				{ name: "b.pdf", size: 3_000_000 },
				{ name: "c.pdf", size: 3_000_000 },
			]),
			mode: "append",
		});
		expect(ofType(driver.last, "upload")).toEqual([]);
		const slots = driver.state.rail.values.receipt as readonly FileSlot[];
		expect(slots.map((slot) => slot.state)).toEqual(["sent", "sent", "sent"]);
		expect(slots[0].ref).toEqual({ kind: "inline" });
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		expect(driver.state.runs).toEqual([]);
		const message = driver.state.view.message?.message;
		expect(message?.kind).toBe("requestTooLarge");
		expect(message?.kind === "requestTooLarge" && message.totalBytes).toBe(
			9_000_000,
		);
	});
});

describe("Run again / Try again", () => {
	test("runs the run's own copy, never touches the rail; the new run takes the stage, the cursor its Stop", () => {
		const driver = from("done");
		driver.command({ type: "setValue", key: "max_pages", value: "33" });
		const rail = driver.state.rail.values;
		driver.advance(1000).command({ type: "runAgain", runId: "run-14" });
		const [run] = driver.state.runs;
		expect(run.n).toBe(15);
		expect(run.copy.values.max_pages).toBe("20");
		expect(driver.state.rail.values).toBe(rail);
		expect(driver.state.view.selectedRunId).toBe(run.id);
		expect(driver.state.view.focus?.target).toEqual({ kind: "stop" });
	});

	test("a run whose files can no longer be sent is not run again", () => {
		const driver = from("reopen");
		driver.command({ type: "runAgain", runId: "run-14" });
		expect(driver.state.runs[0].n).toBe(14);
		expect(driver.last).toEqual([]);
	});

	test("a copy today's form would refuse is not run again (a required field was added since)", () => {
		const done = fixture("done");
		const [first] = done.form.fields.filter((field) => field.kind === "text");
		if (!first) throw new Error("the fixture has no text field");
		const added = {
			...first,
			key: "po_number",
			name: "po_number",
			label: "PO number",
			required: true,
			hasDefault: false,
			defaultValue: "",
			index: done.form.fields.length,
		};
		const driver = sessionDriver(
			{ ...done, form: { ...done.form, fields: [...done.form.fields, added] } },
			fixtureClock("done"),
		);
		driver.advance(1000).command({ type: "runAgain", runId: "run-14" });
		expect(driver.state.runs[0].n).toBe(14);
		expect(driver.last).toEqual([]);
	});

	test("Try again on a run whose file was not sent uploads the File it holds again", () => {
		const driver = from("small");
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r.jpg", size: 10 }]),
			mode: "replace",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		driver.failUpload("pick-r.jpg");
		const [failed] = driver.state.runs;
		expect(failed.status).toBe("notStarted");
		expect(failed.outcome).toEqual({
			kind: "notStarted",
			reason: "fileNotSent",
		});
		driver.command({
			type: "removeFile",
			name: "receipt",
			slotId: "pick-r.jpg",
		});
		driver.advance(1000).command({ type: "runAgain", runId: failed.id });
		const [retry] = driver.state.runs;
		expect(retry.n).toBe(2);
		expect(retry.status).toBe("sending");
		expect(
			ofType(driver.last, "upload").map((effect) => effect.slotId),
		).toEqual(["pick-r.jpg"]);
		driver.sendUploads();
		expect(driver.state.runs[0].status).toBe("starting");
	});
});
