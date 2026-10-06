import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { createElement, useEffect, useRef } from "react";
import {
	type CopyValue,
	FIELD_KEY_SEPARATOR,
	type FieldKind,
	type FieldValue,
	type FileSlot,
	type FormSessionState,
	LIVE_RUN_STATUSES,
	type RunEntry,
	type RunStatus,
	type WorkbenchField,
} from "../contracts";
import { type DomHarness, installWorkbenchDom, mountWorkbench } from "./dom";
import {
	ACTION_NAME_LIST,
	fakeActions,
	fakeRoutes,
	fakeView,
} from "./fake-actions";
import {
	FIXTURE_FIELDS,
	FIXTURE_NAMES,
	FIXTURE_TODAY,
	type FixtureName,
	fixture,
	fixtureClock,
	fixtureMeta,
	fixtureRouteLabels,
} from "./fixtures";
import {
	DESKTOP_LAYOUT,
	PHONE_LAYOUT,
	artboardLayout,
	layoutFor,
	withLayout,
} from "./layouts";

const _fixture: (name: FixtureName) => FormSessionState = fixture;

const all = FIXTURE_NAMES.map((name) => [name, fixture(name)] as const);

const runOf = (state: FormSessionState, n: number) =>
	state.runs.find((run) => run.n === n) as RunEntry;

const isSlot = (value: unknown): value is FileSlot =>
	typeof value === "object" &&
	value !== null &&
	"state" in value &&
	"id" in value;

const isText = (value: unknown) => typeof value === "string";
const isList = (value: unknown, item: (entry: unknown) => boolean) =>
	Array.isArray(value) && value.every(item);
const isObject = (value: unknown) =>
	typeof value === "object" && value !== null && !Array.isArray(value);

/** The shape each kind's rail value has (contracts `FieldValue`). */
const FITS_KIND: Readonly<Record<FieldKind, (value: FieldValue) => boolean>> = {
	text: isText,
	number: isText,
	date: isText,
	choice: isText,
	json: isText,
	bool: (value) => typeof value === "boolean",
	chips: (value) => isList(value, isText),
	file: (value) => value === null || isSlot(value),
	files: (value) => isList(value, isSlot),
	group: isObject,
	pairs: (value) => Array.isArray(value),
	unsupported: () => true,
};

const fitsKind = (field: WorkbenchField, value: FieldValue) =>
	FITS_KIND[field.kind](value);

function fieldKeys(fields: readonly WorkbenchField[]): string[] {
	return fields.flatMap((field) => [field.key, ...fieldKeys(field.props)]);
}

/** Every file slot a state holds: rail values, next files, left-out files, run copies. */
function slotsOf(state: FormSessionState): FileSlot[] {
	const values: unknown[] = [
		...Object.values(state.rail.values),
		...Object.values(state.rail.nextFiles).flat(),
		...Object.values(state.rail.leftOut).flatMap((list) =>
			list.map((file) => file.slot),
		),
		...state.runs.flatMap((run) => Object.values(run.copy.values)),
	];
	return values.flatMap((value) =>
		(Array.isArray(value) ? value : [value]).filter(isSlot),
	);
}

const WAITING: readonly RunStatus[] = ["queued", "sending"];

/** Live runs have not ended; queued and sending ones have not started; history has no output. */
function expectRunTimes(run: RunEntry, now: number) {
	const waiting = WAITING.includes(run.status);
	const history = run.origin === "history";
	expect(run.endedAt === null).toBe(LIVE_RUN_STATUSES.includes(run.status));
	expect(run.startedAt === null).toBe(waiting);
	expect(run.createdAt).toBeLessThanOrEqual(now);
	expect(run.startedAt ?? now).toBeGreaterThanOrEqual(run.createdAt);
	expect(run.endedAt ?? now).toBeLessThanOrEqual(now);
	expect(run.output === null).toBe(history || waiting);
	expect(run.target === null).toBe(history);
}

function hasUndefined(value: unknown): boolean {
	if (value === undefined) return true;
	if (value === null || typeof value !== "object") return false;
	return Object.values(value).some(hasUndefined);
}

describe("every fixture", () => {
	test("names are unique and every name builds", () => {
		expect(new Set(FIXTURE_NAMES).size).toBe(FIXTURE_NAMES.length);
		expect(all.length).toBe(32);
	});

	test.each(all)("%s: run ids and numbers are unique", (_, state) => {
		const ids = state.runs.map((run) => run.id);
		expect(new Set(ids).size).toBe(ids.length);
		expect(new Set(state.runs.map((run) => run.n)).size).toBe(ids.length);
	});

	test.each(all)(
		"%s: runs are newest first, this session's first",
		(_, state) => {
			const origins = state.runs.map((run) => run.origin);
			const firstHistory = origins.indexOf("history");
			if (firstHistory >= 0)
				expect(
					origins.slice(firstHistory).every((origin) => origin === "history"),
				).toBe(true);
			for (let index = 1; index < state.runs.length; index++)
				expect(state.runs[index - 1].createdAt).toBeGreaterThan(
					state.runs[index].createdAt,
				);
		},
	);

	test.each(all)(
		"%s: references point at runs and fields that exist",
		(_, state) => {
			const ids = new Set(state.runs.map((run) => run.id));
			const keys = new Set(fieldKeys(state.form.fields));
			for (const id of [
				state.view.selectedRunId,
				state.view.pinnedRunId,
				state.rail.comparedRunId,
			])
				if (id !== null) expect(ids.has(id)).toBe(true);
			if (state.view.list) expect(keys.has(state.view.list.key)).toBe(true);
			const target = state.view.focus?.target;
			if (target?.kind === "field") expect(keys.has(target.key)).toBe(true);
			for (const key of Object.keys(state.rail.problems))
				expect(keys.has(key)).toBe(true);
			for (const key of Object.keys(state.rail.texts))
				expect(keys.has(key)).toBe(true);
		},
	);

	test.each(all)("%s: rail values fit their fields", (_, state) => {
		const names = state.form.fields.map((field) => field.name);
		expect(Object.keys(state.rail.values).sort()).toEqual([...names].sort());
		for (const field of state.form.fields)
			expect(fitsKind(field, state.rail.values[field.name])).toBe(true);
	});

	test.each(all)("%s: a slot id names one file everywhere", (_, state) => {
		const seen = new Map<string, string>();
		for (const slot of slotsOf(state)) {
			const key = `${slot.name}|${slot.size}`;
			expect(seen.get(slot.id) ?? key).toBe(key);
			seen.set(slot.id, key);
		}
		for (const run of state.runs)
			for (const id of run.pendingSlotIds) expect(seen.has(id)).toBe(true);
	});

	test.each(all)(
		"%s: run times fit the run's status and the clock",
		(name, state) => {
			const { now } = fixtureClock(name);
			for (const run of state.runs) expectRunTimes(run, now);
			for (const slot of slotsOf(state))
				expect(slot.sentAt ?? now).toBeLessThanOrEqual(now);
		},
	);

	test.each(all)("%s: numbers and sequence counters are ahead", (_, state) => {
		const highest = Math.max(0, ...state.runs.map((run) => run.n));
		expect(state.memory.prefs.nextRunNumber).toBe(highest + 1);
		const seqs = [
			state.view.focus?.seq,
			state.view.message?.seq,
			state.announcement?.seq,
			state.undo?.seq,
		].filter((seq): seq is number => seq !== undefined);
		for (const seq of seqs) expect(state.seq).toBeGreaterThanOrEqual(seq);
	});

	test.each(all)("%s: Main's viewer, JSON-safe, desktop layout", (_, state) => {
		expect(state.form.viewer.mac).toBe(true);
		expect(state.form.viewer.locale).toBe("en-GB");
		expect(state.form.viewer.dateLocale.order).toBe("dmy");
		expect(state.form.viewer.dateLocale.sep).toBe("/");
		expect(state.form.viewer.decimalSign).toBe(".");
		expect(state.layout).toEqual(DESKTOP_LAYOUT);
		expect(hasUndefined(state)).toBe(false);
		expect(JSON.parse(JSON.stringify(state))).toEqual(state);
	});

	test.each(all)("%s: meta matches the state", (name, state) => {
		const meta = fixtureMeta(name);
		expect(meta.name).toBe(name);
		expect(state.form.fields).toBe(FIXTURE_FIELDS[meta.form]);
		expect(Object.keys(meta.routeLabels)).toEqual([...state.form.routes]);
	});

	test("each call builds a fresh state", () => {
		const first = fixture("series");
		const second = fixture("series");
		expect(second).toEqual(first);
		expect(second).not.toBe(first);
		expect(second.runs[0]).not.toBe(first.runs[0]);
	});
});

describe("forms", () => {
	test("FlowPath pins are file fields, several files where DATA.js has multi", () => {
		const files = Object.values(FIXTURE_FIELDS)
			.flat()
			.filter((field) => field.fileMode !== null)
			.map(
				(field) =>
					`${field.name}:${field.kind}:${field.dataType}:${field.valueType}`,
			);
		expect(files).toEqual([
			"receipt:file:Struct:Normal",
			"invoice_file:file:Struct:Normal",
			"supporting_documents:files:Struct:Array",
			"baseline_export:file:Struct:Normal",
			"reference_documents:files:Struct:Array",
		]);
		for (const field of Object.values(FIXTURE_FIELDS).flat())
			if (field.fileMode !== null) expect(field.fileMode).toBe("flowpath");
	});

	test("sizes, required fields and keys of the four forms", () => {
		const count = (key: keyof typeof FIXTURE_FIELDS) =>
			FIXTURE_FIELDS[key].length;
		expect([
			count("none"),
			count("small"),
			count("medium"),
			count("large"),
		]).toEqual([0, 3, 9, 29]);
		const required = FIXTURE_FIELDS.large.filter((field) => field.required);
		expect(required.map((field) => field.name)).toEqual([
			"portal_url",
			"customer_number",
			"order_numbers",
			"date_from",
			"report_title",
		]);
		const terms = FIXTURE_FIELDS.medium[8];
		expect(terms.kind).toBe("group");
		expect(terms.required).toBe(false);
		expect(terms.props.map((prop) => prop.key)).toEqual(
			["currency", "net_days", "discount_percent", "discount_days"].map(
				(prop) => `payment_terms${FIELD_KEY_SEPARATOR}${prop}`,
			),
		);
		expect(terms.props[0].options).toEqual(["EUR", "USD", "CHF", "GBP"]);
	});

	test("the dot rule's input: only fields with a default can carry the dot (spec 2.0, §7)", () => {
		const withDefault = (key: keyof typeof FIXTURE_FIELDS) =>
			FIXTURE_FIELDS[key]
				.filter((field) => field.hasDefault)
				.map((field) => field.name);
		expect(withDefault("small")).toEqual(["quantity"]);
		expect(withDefault("medium")).toEqual([
			"expected_total",
			"max_pages",
			"run_ocr",
			"cost_centers",
			"payment_terms",
		]);
		const large = new Set(withDefault("large"));
		for (const field of FIXTURE_FIELDS.large)
			expect(large.has(field.name)).toBe(
				!field.required && field.fileMode === null,
			);
	});

	test("optional fields without a stored default are seeded as today's form does", () => {
		const to = FIXTURE_FIELDS.large.find((field) => field.name === "date_to");
		expect(to?.defaultValue).toBe(FIXTURE_TODAY);
		expect(to?.hasDefault).toBe(true);
		const documents = FIXTURE_FIELDS.medium[1];
		expect(documents.defaultValue).toEqual([]);
		expect(documents.hasDefault).toBe(false);
	});

	test("switches, groups and pairs are never required; short controls are flagged", () => {
		for (const field of Object.values(FIXTURE_FIELDS).flat()) {
			if (field.kind === "bool" || field.kind === "group")
				expect(field.required).toBe(false);
			if (["number", "date", "bool"].includes(field.kind))
				expect(field.short).toBe(true);
			if (["text", "chips", "file", "files", "group"].includes(field.kind))
				expect(field.short).toBe(false);
		}
	});
});

describe("base artboards (spec §7)", () => {
	test.each([
		["running", 14, "running", 31],
		["streaming", 14, "streaming", 44],
		["done", 14, "done", 48],
		["stopped", 14, "stopped", 37],
		["failed", 13, "failed", 12],
		["asking", 14, "asking", 39],
	] as const)(
		"%s shows run %i of this session (%s, %i s)",
		(name, n, status, seconds) => {
			const state = fixture(name);
			const run = runOf(state, n);
			expect(run.origin).toBe("session");
			expect(run.status).toBe(status);
			expect(state.view.selectedRunId).toBe(run.id);
			expect(state.rail.comparedRunId).toBe(run.id);
			expect(run.copy.values).toBe(state.rail.values);
			const end = run.endedAt ?? fixtureClock(name).now;
			expect(Math.floor((end - (run.startedAt ?? 0)) / 1000)).toBe(seconds);
			expect(state.runs.length).toBe(n);
		},
	);

	test("done carries the answer, FL_RESULT and twelve files; failed stops at step 2", () => {
		const done = runOf(fixture("done"), 14).output;
		expect(done?.answer.startsWith("## Invoice RE-2026-0917 extracted")).toBe(
			true,
		);
		expect(done?.attachments.length).toBe(12);
		expect(done?.steps.map((step) => step.state)).toEqual(
			Array(6).fill("done"),
		);
		expect(done?.result).not.toBeNull();
		const failed = runOf(fixture("failed"), 13);
		expect(failed.failedAt).toEqual({ number: 2, title: "Run OCR" });
		expect(failed.output?.steps.map((step) => step.state)).toEqual([
			"done",
			"failed",
		]);
		expect(failed.outcome?.kind).toBe("failed");
		expect(failed.unseenFailure).toBe(false);
	});

	test("streaming has streamed up to the caret; running has no answer yet", () => {
		const streaming = runOf(fixture("streaming"), 14).output;
		expect(streaming?.answer.endsWith("Net € 10,490")).toBe(true);
		expect(streaming?.steps.at(-1)?.state).toBe("active");
		expect(runOf(fixture("running"), 14).output?.answer).toBe("");
	});

	test("compare pins run 12 beside run 14, both with answers", () => {
		const state = fixture("compare");
		expect(state.view.pinnedRunId).toBe("run-12");
		expect(runOf(state, 12).output?.answer).toContain("RE-2026-0911");
		expect(runOf(state, 12).copy.values.run_ocr).toBe(false);
		expect(runOf(state, 14).copy.values.run_ocr).toBe(true);
	});

	test("first visits, zero-field and three-field artboards", () => {
		expect(fixture("idle").runs).toEqual([]);
		expect(fixture("invalid").rail.pressed).toBe(true);
		expect(Object.keys(fixture("invalid").rail.problems)).toEqual([
			"invoice_file",
			"vendor_name",
		]);
		expect(fixture("none").form.fields).toEqual([]);
		expect(runOf(fixture("none-done"), 1).status).toBe("empty");
		const smallDone = fixture("small-done");
		expect(runOf(smallDone, 1).origin).toBe("session");
		expect(smallDone.rail.values.quantity).toBe("2");
		expect(fixture("large").rail.values.customer_number).toBe("K-204418");
		expect(fixture("runs").rail.tab).toBe("runs");
	});

	test("uploading: run 1 waits in Sending for its two files", () => {
		const state = fixture("uploading");
		const run = runOf(state, 1);
		expect(run.status).toBe("sending");
		expect(run.pendingSlotIds).toEqual(["slot-ls-77120", "slot-po-48213"]);
		expect(state.view.message?.message.kind).toBe("start");
	});
});

describe("spec §8 presets", () => {
	test("reopen: fourteen runs from history, run 14 as context", () => {
		const state = fixture("reopen");
		expect(state.runs.every((run) => run.origin === "history")).toBe(true);
		expect(state.view.selectedRunId).toBe("run-14");
		expect(state.rail.comparedRunId).toBeNull();
		const invoice = runOf(state, 14).copy.values.invoice_file as FileSlot;
		expect(invoice.state).toBe("reminder");
	});

	test("next-files: 0918 sending, nine next files, 0917 left out by run 14", () => {
		const state = fixture("next-files");
		const current = state.rail.values.invoice_file as FileSlot;
		expect([current.name, current.state, current.progress]).toEqual([
			"invoice-RE-2026-0918.pdf",
			"sending",
			0.4,
		]);
		const next = state.rail.nextFiles.invoice_file;
		expect(next.length).toBe(9);
		expect(next.map((slot) => slot.state)).toEqual([
			"sending",
			...Array(8).fill("waiting"),
		]);
		expect(state.rail.leftOut.invoice_file.map((file) => file.n)).toEqual([14]);
		expect(state.view.list).toEqual({
			kind: "nextFiles",
			key: "invoice_file",
			active: -1,
		});
		expect(state.memory.prefs.auto).toEqual([
			"invoice_file",
			"supporting_documents",
			"invoice_date",
		]);
	});

	test("recent: the list is open under Vendor", () => {
		const state = fixture("recent");
		expect(state.view.list).toEqual({
			kind: "recent",
			key: "vendor_name",
			active: 0,
		});
		expect(state.rail.values.vendor_name).toBe("");
	});

	test("series: runs 15–17 going, 18 queued with its start message", () => {
		const state = fixture("series");
		expect(state.runs.slice(0, 4).map((run) => [run.n, run.status])).toEqual([
			[18, "queued"],
			[17, "running"],
			[16, "running"],
			[15, "running"],
		]);
		expect(state.view.selectedRunId).toBe("run-17");
		const message = state.view.message?.message;
		expect(message?.kind === "start" && message.start.pairs).toEqual([
			"invoice-RE-2026-0921.pdf",
			"22 Sep 2026",
		]);
		expect(state.rail.nextFiles.invoice_file.length).toBe(5);
		expect(runOf(state, 18).copy.values.invoice_date).toBe("2026-09-22");
		const now = fixtureClock("series").now;
		const elapsed = [15, 16, 17].map((n) =>
			Math.floor((now - (runOf(state, n).startedAt ?? now)) / 1000),
		);
		expect(elapsed).toEqual([31, 21, 11]);
	});

	test("series-failed: run 18 failed unseen, 19–21 going, 22 and 23 queued, '30' typed", () => {
		const state = fixture("series-failed");
		const run18 = runOf(state, 18);
		expect(run18.status).toBe("failed");
		expect(run18.unseenFailure).toBe(true);
		expect(run18.failedAt).toEqual({ number: 2, title: "Run OCR" });
		expect(
			state.runs.filter((run) => run.status === "queued").map((run) => run.n),
		).toEqual([23, 22]);
		expect(state.rail.texts.invoice_date).toBe("30");
		expect(state.rail.nextFiles).toEqual({});
		expect(state.view.message).toBeNull();
	});

	test("queued, after-run, shortcuts, preset-save, leave are moments of the series", () => {
		const queued = fixture("queued");
		expect(queued.view.selectedRunId).toBe("run-18");
		expect(queued.rail.comparedRunId).toBe("run-18");
		expect(queued.view.message).toBeNull();
		expect(fixture("after-run").view.overlay).toEqual({
			id: "afterRun",
			focusName: null,
		});
		expect(fixture("shortcuts").view.overlay).toEqual({ id: "shortcuts" });
		expect(fixture("preset-save").view.overlay?.id).toBe("presetSave");
		expect(fixture("leave").view.overlay).toEqual({
			id: "leave",
			route: "/",
			replace: false,
		});
	});

	test("small-offer asks; small-reset made both fields per run", () => {
		const offer = fixture("small-offer");
		expect(offer.view.question).toEqual({
			kind: "offer",
			names: ["order", "receipt"],
		});
		expect(offer.runs.map((run) => run.copy.replaced)).toEqual([
			["order"],
			["order"],
			["order"],
		]);
		const reset = fixture("small-reset");
		expect(reset.memory.prefs.perRun).toEqual(["order", "receipt"]);
		expect(reset.memory.prefs.offerAnswered).toBe(true);
		expect(reset.rail.values.order).toBe("");
		expect(reset.rail.values.receipt).toBeNull();
	});

	test("none-cap: three runs going and the refused press announced", () => {
		const state = fixture("none-cap");
		expect(state.runs.map((run) => run.status)).toEqual([
			"running",
			"running",
			"running",
		]);
		expect(state.announcement).toEqual({
			seq: 2,
			kind: "capReached",
			running: 3,
		});
		expect(state.queue.cap).toBe(3);
	});

	test("hosted-files runs on a hosted link; every other fixture on the desktop app", () => {
		for (const [name, state] of all) {
			const hosted = name === "hosted-files";
			expect(state.form.host.kind).toBe(hosted ? "hosted" : "app");
			expect(state.queue.target).toBe(hosted ? "remote" : "local");
			expect(state.queue.cap).toBe(hosted ? 2 : 3);
		}
		expect(fixture("hosted-files").form.host.flowPathFiles).toBe(false);
	});

	test("presets: Nordwind applied on open, menu open; pick-again brings run 9 back", () => {
		const presets = fixture("presets");
		expect(
			presets.memory.presets.map((preset) => [preset.name, preset.digit]),
		).toEqual([
			["Alpenfracht AG", 1],
			["Nordwind Logistik GmbH", 2],
		]);
		expect(presets.rail.activePresetId).toBe("preset-nordwind");
		const pick = fixture("pick-again");
		const invoice = pick.rail.values.invoice_file as FileSlot;
		expect([invoice.name, invoice.state]).toEqual([
			"invoice-RE-2026-0902.pdf",
			"reminder",
		]);
		expect((pick.rail.values.supporting_documents as FileSlot[]).length).toBe(
			2,
		);
		expect(pick.rail.values.max_pages).toBe("40");
		expect(pick.undo?.kind).toBe("useInputs");
		expect(pick.view.message?.undo).toBe(true);
	});

	test("asking: a pending in-run question", () => {
		const run = runOf(fixture("asking"), 14);
		expect(run.output?.interactions.map((question) => question.status)).toEqual(
			["pending"],
		);
	});

	test("history copies hold reminders, never sent slots", () => {
		for (const [, state] of all)
			for (const run of state.runs.filter(
				(entry) => entry.origin === "history",
			))
				for (const value of Object.values(run.copy.values) as CopyValue[])
					for (const slot of (Array.isArray(value) ? value : [value]).filter(
						isSlot,
					))
						expect(slot.state).toBe("reminder");
	});
});

describe("fake actions and routes", () => {
	test("every action records its arguments", () => {
		const fake = fakeActions();
		fake.actions.setValue("vendor_name", "Nordwind Logistik GmbH", "replace");
		fake.actions.run({ from: "chord" });
		fake.actions.stop("run-17");
		expect(fake.calls.map((call) => call.name)).toEqual([
			"setValue",
			"run",
			"stop",
		]);
		expect(fake.argsOf("stop")).toEqual([["run-17"]]);
		for (const name of ACTION_NAME_LIST)
			expect(typeof fake.actions[name]).toBe("function");
		expect(ACTION_NAME_LIST.length).toBe(49);
		fake.clear();
		expect(fake.calls).toEqual([]);
	});

	test("onCall sees each call; routes record go", () => {
		const seen: string[] = [];
		const fake = fakeActions({ onCall: (call) => seen.push(call.name) });
		fake.actions.setLayout(PHONE_LAYOUT);
		expect(seen).toEqual(["setLayout"]);
		const routes = fakeRoutes(fixtureRouteLabels("medium"));
		routes.go("/review");
		expect(routes.gone).toEqual(["/review"]);
		expect(routes.labels["/"]).toBe("Home");
	});

	test("fakeView writes the layout into the state it hands out", () => {
		const view = fakeView(fixture("done"), { layout: PHONE_LAYOUT });
		expect(view.props.layout).toBe(PHONE_LAYOUT);
		expect(view.props.state.layout).toBe(PHONE_LAYOUT);
		view.props.actions.markSeen("run-13");
		expect(view.calls).toEqual([{ name: "markSeen", args: ["run-13"] }]);
	});
});

describe("layouts", () => {
	test("split from 900, compare from a 600 px stage, pointer flags apart from width", () => {
		expect(layoutFor(1360, 836)).toEqual({
			width: 1360,
			height: 836,
			split: true,
			touch: false,
			finePointer: true,
			compare: true,
		});
		expect(layoutFor(950, 800).compare).toBe(false);
		expect(layoutFor(1000, 800).compare).toBe(true);
		expect(layoutFor(899, 800).split).toBe(false);
		expect(layoutFor(820, 900, { touch: false }).finePointer).toBe(true);
		expect(layoutFor(390, 728, { touch: true }).finePointer).toBe(false);
	});

	test("artboard boxes leave room for the host chrome", () => {
		expect([DESKTOP_LAYOUT.width, DESKTOP_LAYOUT.height]).toEqual([1360, 836]);
		expect([
			PHONE_LAYOUT.width,
			PHONE_LAYOUT.height,
			PHONE_LAYOUT.touch,
		]).toEqual([390, 728, true]);
		expect(artboardLayout(390, 844, { touch: false }).finePointer).toBe(true);
		expect(withLayout(fixture("idle"), PHONE_LAYOUT).layout).toBe(PHONE_LAYOUT);
	});
});

describe("mountWorkbench", () => {
	let dom: DomHarness;
	beforeAll(() => {
		dom = installWorkbenchDom();
	});
	afterAll(() => dom.restore());

	function Probe(props: { readonly sizes: string[] }) {
		const ref = useRef<HTMLDivElement>(null);
		useEffect(() => {
			const observer = new ResizeObserver((entries) => {
				for (const entry of entries)
					props.sizes.push(
						`${entry.contentRect.width}x${entry.contentRect.height}`,
					);
			});
			if (ref.current) observer.observe(ref.current);
			return () => observer.disconnect();
		}, [props.sizes]);
		return createElement("div", { ref, "data-fw-root": "" });
	}

	test("the root reports the layout's box and the pointer queries answer from it", async () => {
		const sizes: string[] = [];
		const view = await mountWorkbench(createElement(Probe, { sizes }), {
			layout: PHONE_LAYOUT,
		});
		expect(sizes).toEqual(["390x728"]);
		expect(window.matchMedia("(pointer: coarse)").matches).toBe(true);
		expect(window.matchMedia("(pointer: fine)").matches).toBe(false);
		expect(CSS.escape("field:a b")).toBe("field\\:a\\ b");
		await view.resize(DESKTOP_LAYOUT);
		expect(sizes).toEqual(["390x728", "1360x836"]);
		expect(matchMedia("( pointer : fine )").matches).toBe(true);
		await view.unmount();
	});
});
