import { describe, expect, test } from "bun:test";
import type {
	FieldMarkers,
	FormSessionState,
	RunEntry,
	WorkbenchField,
} from "../contracts";
import type { FormWords } from "../model/date-text";
import { fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT, layoutFor } from "../testing/layouts";
import {
	NO_MARKERS,
	anyFieldDiffers,
	changeText,
	changedSet,
	columnsOf,
	dayHeading,
	fieldIndex,
	fieldsMatching,
	filterCounts,
	fitsHalfRow,
	focusOwnerName,
	groupRunsByDay,
	isFiltering,
	isIntroduced,
	localDay,
	localDayNumber,
	markerMap,
	ordinalSuffix,
	perRunCount,
	previewOf,
	railShapeOf,
	railValuesOf,
	secretCheck,
	showsPresets,
	waitOf,
} from "./rail-model";
import { runChangesOf } from "./run-text";

const WORDS: FormWords = {
	none: "none",
	empty: "empty",
	on: "On",
	off: "Off",
	files: (count) => `${count} files`,
	entries: (count) => `${count} entries`,
	date: (iso) => iso,
	today: "Today",
	yesterday: "Yesterday",
};

const withRuns = (
	state: FormSessionState,
	runs: readonly RunEntry[],
): FormSessionState => ({ ...state, runs });

const field = (patch: Partial<WorkbenchField>): WorkbenchField => ({
	key: "f",
	name: "f",
	label: "Field",
	help: null,
	kind: "text",
	dataType: "String",
	valueType: "Normal",
	required: false,
	sensitive: false,
	defaultOmitted: false,
	defaultValue: "",
	hasDefault: false,
	options: null,
	range: null,
	step: null,
	integer: false,
	fileMode: null,
	itemKind: null,
	dateFormat: null,
	props: [],
	short: false,
	index: 0,
	...patch,
});

describe("railShapeOf", () => {
	test("a first visit shows the field list alone: no tabs, no filter, no description (the stage has it)", () => {
		const shape = railShapeOf(fixture("idle"), DESKTOP_LAYOUT);
		expect(shape).toEqual({
			tabs: false,
			tab: "inputs",
			filter: false,
			presets: false,
			description: "hidden",
			afterRun: false,
		});
	});

	test("with history the head switches Inputs | Runs and clamps the description", () => {
		const shape = railShapeOf(fixture("done"), DESKTOP_LAYOUT);
		expect(shape.tabs).toBe(true);
		expect(shape.description).toBe("clamped");
		expect(shape.tab).toBe("inputs");
	});

	test("the rail tab follows the state only while the tabs exist", () => {
		expect(railShapeOf(fixture("runs"), DESKTOP_LAYOUT).tab).toBe("runs");
		expect(railShapeOf(fixture("runs"), PHONE_LAYOUT).tab).toBe("inputs");
		const one = withRuns(fixture("runs"), fixture("runs").runs.slice(0, 1));
		expect(railShapeOf(one, DESKTOP_LAYOUT).tab).toBe("inputs");
	});

	test("one run: the description is whole, nothing is clamped", () => {
		const state = fixture("small-done");
		expect(railShapeOf(state, DESKTOP_LAYOUT).description).toBe("full");
	});

	test("the first run's files sending keep the stage empty, so the description stays there", () => {
		expect(railShapeOf(fixture("uploading"), DESKTOP_LAYOUT).description).toBe(
			"hidden",
		);
	});

	test("a narrow box always shows the description", () => {
		expect(railShapeOf(fixture("idle"), PHONE_LAYOUT).description).toBe("full");
	});

	test("the filter row needs twelve fields and the Inputs tab", () => {
		expect(railShapeOf(fixture("large"), DESKTOP_LAYOUT).filter).toBe(true);
		expect(railShapeOf(fixture("idle"), DESKTOP_LAYOUT).filter).toBe(false);
		const runsTab = { ...fixture("large"), runs: fixture("runs").runs };
		const onRuns = {
			...runsTab,
			rail: { ...runsTab.rail, tab: "runs" as const },
		};
		expect(railShapeOf(onRuns, DESKTOP_LAYOUT).filter).toBe(false);
	});

	test("the after-run row exists on a narrow box once the setting was introduced", () => {
		expect(railShapeOf(fixture("series"), PHONE_LAYOUT).afterRun).toBe(true);
		expect(railShapeOf(fixture("series"), DESKTOP_LAYOUT).afterRun).toBe(false);
		expect(railShapeOf(fixture("done"), PHONE_LAYOUT).afterRun).toBe(false);
	});

	test("a box under 900 px with a fine pointer keeps the tabs away but not the pointer habits", () => {
		const narrow = layoutFor(820, 700);
		expect(narrow.split).toBe(false);
		expect(narrow.touch).toBe(false);
		expect(railShapeOf(fixture("runs"), narrow).tabs).toBe(false);
	});
});

describe("showsPresets (S1)", () => {
	test("a preset on this device", () => {
		expect(showsPresets(fixture("presets"))).toBe(true);
	});

	test("twelve fields and two runs", () => {
		const large = fixture("large");
		expect(showsPresets(large)).toBe(false);
		const two = withRuns(large, fixture("done").runs.slice(0, 2));
		expect(showsPresets(two)).toBe(true);
		const one = withRuns(large, fixture("done").runs.slice(0, 1));
		expect(showsPresets(one)).toBe(false);
	});

	test("a medium form without presets never shows it", () => {
		expect(showsPresets(fixture("done"))).toBe(false);
	});
});

describe("introduced", () => {
	test("the saved flag, a per-run field, waiting next files or a question", () => {
		expect(isIntroduced(fixture("series"))).toBe(true);
		expect(isIntroduced(fixture("small-reset"))).toBe(true);
		expect(isIntroduced(fixture("small-offer"))).toBe(true);
		expect(isIntroduced(fixture("done"))).toBe(false);
	});

	test("per-run settings of fields the form no longer has are not counted", () => {
		const state = fixture("small-reset");
		expect(perRunCount(state)).toBe(2);
		const gone = {
			...state,
			memory: {
				...state.memory,
				prefs: { ...state.memory.prefs, perRun: ["order", "ghost"] },
			},
		};
		expect(perRunCount(gone)).toBe(1);
	});
});

describe("markers and Reset to defaults", () => {
	test("nothing differs on a first visit", () => {
		const state = fixture("idle");
		const markers = markerMap(state, fieldIndex(state.form.fields));
		expect(anyFieldDiffers(state.form.fields, markers)).toBe(false);
	});

	test("a typed required field counts, a per-run one too", () => {
		const done = fixture("done");
		const markers = markerMap(done, fieldIndex(done.form.fields));
		expect(anyFieldDiffers(done.form.fields, markers)).toBe(true);
		expect(markers.get("expected_total")?.changed).toBe(true);
		expect(markers.get("vendor_name")?.changed).toBe(false);
	});

	test("object properties have their own markers", () => {
		const medium = fixture("idle");
		const index = fieldIndex(medium.form.fields);
		const group = medium.form.fields.find((item) => item.kind === "group");
		expect(group?.props.length).toBeGreaterThan(0);
		const markers = markerMap(medium, index);
		expect(markers.has(group?.key ?? "")).toBe(true);
		for (const prop of group?.props ?? [])
			expect(markers.has(prop.key)).toBe(true);
	});
});

describe("the filter", () => {
	const large = fixture("large");
	const fields = large.form.fields;
	const changed = changedSet(large);
	const all = { query: "", chip: "all" as const };

	test("counts: all, required, changed", () => {
		const counts = filterCounts(fields, changed);
		expect(counts.all).toBe(29);
		expect(counts.required).toBe(5);
		expect(counts.changed).toBe(0);
	});

	test("the chip then the text, in form order", () => {
		expect(fieldsMatching(fields, all, changed)).toHaveLength(29);
		const required = fieldsMatching(
			fields,
			{ ...all, chip: "required" },
			changed,
		);
		expect(required.every((item) => item.required)).toBe(true);
		expect(required).toHaveLength(5);
		const found = fieldsMatching(
			fields,
			{ ...all, query: "customer" },
			changed,
		);
		expect(found.map((item) => item.label)).toContain("Customer number");
	});

	test("the text matches the payload name and ignores case and accents", () => {
		const sample = [
			field({ key: "a", name: "customer_number", label: "Kundennummer" }),
			field({ key: "b", name: "other", label: "Größe" }),
		];
		expect(
			fieldsMatching(sample, { ...all, query: "CUSTOMER" }, new Set()).map(
				(item) => item.name,
			),
		).toEqual(["customer_number"]);
		expect(
			fieldsMatching(sample, { ...all, query: "grosse" }, new Set()),
		).toEqual([]);
		expect(
			fieldsMatching(sample, { ...all, query: "größe" }, new Set()).map(
				(item) => item.name,
			),
		).toEqual(["other"]);
	});

	test("the changed chip keeps the fields that carry the dot", () => {
		const sample = [field({ name: "a" }), field({ key: "b", name: "b" })];
		const kept = fieldsMatching(
			sample,
			{ ...all, chip: "changed" },
			new Set(["b"]),
		);
		expect(kept.map((item) => item.name)).toEqual(["b"]);
	});

	test("filtering means a chip other than All or any typed text", () => {
		expect(isFiltering(all)).toBe(false);
		expect(isFiltering({ query: "  ", chip: "all" })).toBe(false);
		expect(isFiltering({ query: "x", chip: "all" })).toBe(true);
		expect(isFiltering({ query: "", chip: "required" })).toBe(true);
	});
});

describe("focusOwnerName", () => {
	test("a field target names its top-level field, a property its object", () => {
		const state = fixture("invalid");
		const at = (key: string) =>
			({
				view: {
					...state.view,
					focus: {
						seq: 1,
						target: { kind: "field", key, select: false, scrollOnly: false },
					},
				},
			}) as Pick<FormSessionState, "view">;
		expect(focusOwnerName(at("vendor_name"))).toBe("vendor_name");
		expect(focusOwnerName(at("payment_terms\u001fdays"))).toBe("payment_terms");
		expect(focusOwnerName({ view: { ...state.view, focus: null } })).toBeNull();
		expect(
			focusOwnerName({
				view: { ...state.view, focus: { seq: 2, target: { kind: "run" } } },
			}),
		).toBeNull();
	});
});

describe("the grid", () => {
	const quiet: FieldMarkers = NO_MARKERS;
	const optional: FieldMarkers = { ...NO_MARKERS, optional: true };

	test("only short fields can share a row", () => {
		expect(fitsHalfRow(field({ label: "Name", short: false }), quiet)).toBe(
			false,
		);
		expect(
			fitsHalfRow(field({ label: "Max pages", short: true }), optional),
		).toBe(true);
	});

	test("a label that would be cut takes the whole row (fix report)", () => {
		for (const label of [
			"Page load timeout (s)",
			"Slow page threshold (s)",
			"Minimum quality score",
			"Tolerance percent",
		])
			expect(fitsHalfRow(field({ label, short: true }), optional)).toBe(false);
		for (const label of ["Invoice date", "Expected total", "Max pages"])
			expect(fitsHalfRow(field({ label, short: true }), optional)).toBe(true);
	});

	test("without Optional there is more room", () => {
		const label = "Minimum quality score";
		expect(fitsHalfRow(field({ label, short: true }), quiet)).toBe(true);
	});

	test("a single-column rail gives every field its own row", () => {
		const short = field({ label: "Max pages", short: true });
		expect(columnsOf(short, quiet, true)).toBe(1);
		expect(columnsOf(short, quiet, false)).toBe(2);
		expect(columnsOf(field({ label: "Vendor" }), quiet, true)).toBe(2);
	});

	test("a field does not change rows when it becomes per run", () => {
		const short = field({ label: "Invoice date", short: true });
		const perRun: FieldMarkers = { ...NO_MARKERS, perRun: "perRun" };
		expect(columnsOf(short, perRun, true)).toBe(columnsOf(short, quiet, true));
	});
});

describe("runs by day", () => {
	test("history groups under its days, newest first", () => {
		const state = fixture("runs");
		const now = Date.now();
		const groups = groupRunsByDay(state.runs, state.runs[0]?.createdAt ?? now);
		const counts = groups.map((group) => group.runs.length);
		expect(counts.reduce((sum, count) => sum + count, 0)).toBe(14);
		expect(groups.length).toBeGreaterThanOrEqual(3);
		expect(groups[0]?.runs[0]?.n).toBe(14);
	});

	test("queued and sending runs head Today whatever their day", () => {
		const state = fixture("series");
		const now = state.runs[0]?.createdAt ?? Date.now();
		const queued = state.runs.filter((run) => run.status === "queued");
		expect(queued.length).toBeGreaterThan(0);
		const groups = groupRunsByDay(state.runs, now);
		const first = groups[0];
		expect(
			first?.runs
				.slice(0, queued.length)
				.every((run) => run.status === "queued"),
		).toBe(true);
		expect(first?.day).toBe(localDayNumber(now));
		const total = groups.reduce((sum, group) => sum + group.runs.length, 0);
		expect(total).toBe(state.runs.length);
	});

	test("a queued run from yesterday still sits under Today", () => {
		const state = fixture("series");
		const now = Date.now();
		const queued = state.runs.find((run) => run.status === "queued");
		if (!queued) throw new Error("fixture has no queued run");
		const old: RunEntry = { ...queued, createdAt: now - 3 * 86_400_000 };
		const groups = groupRunsByDay([old], now);
		expect(groups).toHaveLength(1);
		expect(groups[0]?.day).toBe(localDayNumber(now));
	});

	test("headings read Today, Yesterday and a dated day", () => {
		const now = new Date(2026, 9, 5, 14, 2).getTime();
		const at = (day: number) => new Date(2026, 8, day, 10, 0).getTime();
		expect(
			dayHeading(new Date(2026, 9, 5, 9, 0).getTime(), now, "en-GB", WORDS),
		).toBe("Today");
		expect(
			dayHeading(new Date(2026, 9, 4, 17, 0).getTime(), now, "en-GB", WORDS),
		).toBe("Yesterday");
		expect(dayHeading(at(28), now, "en-GB", WORDS)).toBe("Mon 28 Sep");
	});

	test("localDay is the viewer's calendar day", () => {
		expect(localDay(new Date(2026, 0, 5, 23, 59))).toBe("2026-01-05");
		expect(localDay(new Date(2026, 11, 31, 0, 0))).toBe("2026-12-31");
	});
});

describe("ordinals", () => {
	test("English suffixes", () => {
		const suffixes = [
			1, 2, 3, 4, 10, 11, 12, 13, 21, 22, 23, 101, 111, 112,
		].map((n) => `${n}${ordinalSuffix(n)}`);
		expect(suffixes).toEqual([
			"1st",
			"2nd",
			"3rd",
			"4th",
			"10th",
			"11th",
			"12th",
			"13th",
			"21st",
			"22nd",
			"23rd",
			"101st",
			"111th",
			"112th",
		]);
	});
});

describe("what a row says", () => {
	const state = fixture("runs");
	const isSecret = secretCheck([]);
	const context = { fields: state.form.fields, words: WORDS, isSecret };
	const asTyped = (text: string) => text;
	const changesOf = (
		run: RunEntry,
		older: RunEntry | undefined,
		secret = isSecret,
	) => runChangesOf(run, older, { ...context, isSecret: secret }, asTyped);

	test("the first run has nothing to compare with", () => {
		const oldest = state.runs.at(-1) as RunEntry;
		expect(changesOf(oldest, undefined)).toEqual({ kind: "first" });
	});

	test("changes against the run before, from → to, field order", () => {
		const run14 = state.runs[0] as RunEntry;
		const run13 = state.runs[1] as RunEntry;
		const changes = changesOf(run14, run13);
		expect(changes.kind).toBe("diff");
		if (changes.kind !== "diff") return;
		expect(changeText(changes.items)).toContain("→");
		expect(changes.items.every((item) => item.label !== "")).toBe(true);
	});

	test("the same inputs name the run before", () => {
		const run = state.runs[0] as RunEntry;
		const changes = changesOf(run, { ...run, n: 9, id: "other" });
		expect(changes).toEqual({ kind: "same", n: 9 });
	});

	test("a secret field reads as the mask on both sides", () => {
		const vendor = state.form.fields.find(
			(item) => item.name === "vendor_name",
		);
		if (!vendor) throw new Error("no vendor field");
		const run = state.runs[0] as RunEntry;
		const other: RunEntry = {
			...run,
			id: "x",
			copy: {
				...run.copy,
				values: { ...run.copy.values, vendor_name: "Alpha" },
			},
		};
		const mine: RunEntry = {
			...run,
			copy: {
				...run.copy,
				values: { ...run.copy.values, vendor_name: "Beta" },
			},
		};
		const masked = changesOf(mine, other, secretCheck(["vendor_name"]));
		expect(masked.kind).toBe("diff");
		if (masked.kind !== "diff") return;
		const item = masked.items.find((entry) => entry.label === vendor.label);
		expect(item).toMatchObject({
			label: vendor.label,
			from: "••••",
			to: "••••",
		});
	});

	test("hidden values read back from storage count as secrets and as no rail value", () => {
		const hidden = { vendor_name: { $hidden: true as const }, max_pages: "20" };
		expect(railValuesOf(hidden)).toEqual({
			vendor_name: null,
			max_pages: "20",
		});
	});
});

describe("the third line", () => {
	const base = fixture("runs").runs[0] as RunEntry;
	const run = (patch: Partial<RunEntry>): RunEntry => ({ ...base, ...patch });
	const step = { number: 2, title: "Run OCR" };

	test("a done run shows the first line of its answer", () => {
		const done = run({
			status: "done",
			summary: { ...base.summary, firstLine: "Invoice extracted." },
		});
		expect(previewOf(done)).toEqual({
			kind: "text",
			text: "Invoice extracted.",
		});
	});

	test("a failed run names its step, else its first line", () => {
		const failed = run({ status: "failed", failedAt: step });
		expect(previewOf(failed)).toEqual({ kind: "failedAt", step });
		const bare = run({
			status: "failed",
			failedAt: null,
			summary: { ...base.summary, stepReached: null, firstLine: "Boom" },
		});
		expect(previewOf(bare)).toEqual({ kind: "text", text: "Boom" });
	});

	test("a stopped run names the step it reached", () => {
		const stopped = run({
			status: "stopped",
			failedAt: null,
			summary: { ...base.summary, stepReached: step },
		});
		expect(previewOf(stopped)).toEqual({ kind: "stoppedAt", step });
	});

	test("not started says why; unknown says the page closed", () => {
		const outcome = { kind: "notStarted", reason: "formClosed" } as const;
		expect(previewOf(run({ status: "notStarted", outcome }))).toEqual({
			kind: "notStarted",
			reason: "formClosed",
		});
		expect(previewOf(run({ status: "unknown" }))).toEqual({ kind: "unknown" });
	});

	test("a finished run that returned nothing says so", () => {
		const empty = run({
			status: "empty",
			summary: { ...base.summary, firstLine: null },
		});
		expect(previewOf(empty)).toEqual({ kind: "nothing" });
	});

	test("a running run with no output has no third line", () => {
		const live = run({
			status: "running",
			summary: { ...base.summary, firstLine: null },
		});
		expect(previewOf(live)).toBeNull();
	});
});

describe("the wait", () => {
	const queued = fixture("series").runs.find(
		(item) => item.status === "queued",
	) as RunEntry;

	test("a queued run names its place, a refused one the missing place", () => {
		expect(waitOf(queued, 1)).toEqual({ kind: "place", place: 1 });
		expect(waitOf({ ...queued, waitingForPlace: true }, 1)).toEqual({
			kind: "noPlace",
		});
	});

	test("only queued runs wait", () => {
		expect(waitOf({ ...queued, status: "running" }, 1)).toBeNull();
		expect(waitOf(queued, undefined)).toBeNull();
	});
});
