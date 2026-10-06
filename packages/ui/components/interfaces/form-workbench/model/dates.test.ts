import { describe, expect, test } from "bun:test";
import {
	type CopyValue,
	type DateLocale,
	FIELD_KEY_SEPARATOR,
	type FieldKind,
	type FormPrefs,
	type FormSessionState,
	type HostCapabilities,
	type RailState,
	type ReadDate,
	type RunEntry,
	type ViewState,
	type ViewerHabits,
	type WorkbenchField,
} from "../contracts";
import {
	dateAnchorOf,
	dateExample,
	dateLocaleOf,
	isoParts,
	readDate,
	shiftIso,
} from "./dates";

const _contract: ReadDate = readDate;

const GB = dateLocaleOf("en-GB");
const US = dateLocaleOf("en-US");
const DE = dateLocaleOf("de");
const ES = dateLocaleOf("es");
const FR = dateLocaleOf("fr");
const PL = dateLocaleOf("pl");
const JA = dateLocaleOf("ja");
const KO = dateLocaleOf("ko");
const ZH = dateLocaleOf("zh");

/** The benchmark's day: run 14 holds 17 Sep 2026, "today" is Wed 30 Sep 2026. */
const TODAY = "2026-09-30";
const RUN_14 = "2026-09-17";

function read(
	text: string,
	anchor: string | null = RUN_14,
	locale: DateLocale = GB,
	today = TODAY,
) {
	return _contract(text, anchor, today, locale);
}

const isoOf = (
	text: string,
	anchor: string | null = RUN_14,
	locale: DateLocale = GB,
) => read(text, anchor, locale)?.iso ?? null;

describe("dateLocaleOf", () => {
	test("the order of day, month and year comes from Intl", () => {
		for (const locale of [GB, DE, ES, FR, PL, dateLocaleOf("pt-BR")])
			expect(locale.order).toBe("dmy");
		expect(US.order).toBe("mdy");
		for (const locale of [JA, KO, ZH]) expect(locale.order).toBe("ymd");
	});

	test("the separator of numeric dates", () => {
		expect(GB.sep).toBe("/");
		expect(US.sep).toBe("/");
		expect(DE.sep).toBe(".");
		expect(PL.sep).toBe(".");
		expect(JA.sep).toBe("/");
		expect(KO.sep).toBe(".");
	});

	test("month names in the viewer's language and English, folded", () => {
		expect(GB.months[8]).toEqual(expect.arrayContaining(["september", "sep"]));
		expect(DE.months[8]).toEqual(expect.arrayContaining(["september", "sept"]));
		expect(ES.months[7]).toEqual(
			expect.arrayContaining(["august", "agosto", "ago"]),
		);
		expect(PL.months[8]).toEqual(
			expect.arrayContaining(["september", "wrzesien", "wrzesnia"]),
		);
		expect(JA.months[8]).toEqual(["september"]);
	});

	test("today, yesterday and tomorrow in the viewer's language, then English", () => {
		expect(GB.words.today).toEqual(["today"]);
		expect(DE.words.today).toEqual(["heute", "today"]);
		expect(DE.words.yesterday).toEqual(["gestern", "yesterday"]);
		expect(ES.words.tomorrow).toEqual(["manana", "tomorrow"]);
		expect(JA.words.today).toEqual(["今日", "today"]);
	});

	test("an unknown tag still gives habits", () => {
		const unknown = dateLocaleOf("not a locale!");
		expect(["dmy", "mdy", "ymd"]).toContain(unknown.order);
		expect(unknown.months).toHaveLength(12);
	});
});

describe("readDate: the spec's readings", () => {
	test("empty text reads as empty", () => {
		expect(read("")).toEqual({ iso: "", past: 0 });
		expect(read("   ")).toEqual({ iso: "", past: 0 });
	});

	test("the benchmark: 18 reads as Fri 18 Sep 2026 against run 14", () => {
		expect(read("18")).toEqual({ iso: "2026-09-18", past: 0 });
		expect(isoOf("18.")).toBe("2026-09-18");
	});

	test("2/10 reads as Fri 2 Oct 2026; 31 cannot be read in September", () => {
		expect(isoOf("2/10")).toBe("2026-10-02");
		expect(read("31")).toBeNull();
	});

	test("today, yesterday and tomorrow; three letters are enough", () => {
		expect(isoOf("today")).toBe(TODAY);
		expect(isoOf("Tod")).toBe(TODAY);
		expect(isoOf("yesterday")).toBe("2026-09-29");
		expect(isoOf("tom")).toBe("2026-10-01");
		expect(read("to")).toBeNull();
		expect(isoOf("heute", RUN_14, DE)).toBe(TODAY);
		expect(isoOf("today", RUN_14, DE)).toBe(TODAY);
		expect(isoOf("gestern", RUN_14, DE)).toBe("2026-09-29");
		expect(isoOf("mañana", RUN_14, ES)).toBe("2026-10-01");
		expect(isoOf("今日", RUN_14, JA)).toBe(TODAY);
		expect(isoOf("明日", RUN_14, JA)).toBe("2026-10-01");
	});

	test("a day alone far before the anchor carries the warning: after 30 Sep, 1 is 29 days back", () => {
		expect(read("1", "2026-09-30")).toEqual({ iso: "2026-09-01", past: 29 });
		expect(read("10", "2026-09-30")).toEqual({ iso: "2026-09-10", past: 20 });
		expect(read("16", "2026-09-30")).toEqual({ iso: "2026-09-16", past: 0 });
		expect(read("30", "2026-09-30")).toEqual({ iso: "2026-09-30", past: 0 });
	});

	test("day and month take the year nearest the anchor: 3.1 after 28 Dec 2026 is 3 Jan 2027", () => {
		expect(isoOf("3.1", "2026-12-28")).toBe("2027-01-03");
		expect(isoOf("28.12", "2027-01-02")).toBe("2026-12-28");
	});

	test("two-digit years take the century nearest to today", () => {
		expect(isoOf("1.2.85")).toBe("1985-02-01");
		expect(isoOf("18/9/26")).toBe("2026-09-18");
		expect(isoOf("1.1.76")).toBe("1976-01-01");
	});

	test("day first: every way of writing 18 September", () => {
		for (const text of [
			"18/9",
			"18.9",
			"18-9",
			"18 9",
			"1809",
			"18/9/26",
			"18.9.2026",
			"180926",
			"18092026",
			"2026/9/18",
			"2026-09-18",
			"18 sep",
			"sep 18",
			"18 September 2026",
			"September 18, 2026",
		])
			expect(isoOf(text)).toBe("2026-09-18");
	});

	test("month first and year first read 9/18", () => {
		for (const text of ["9/18", "9/18/26", "09182026", "Sep 18"])
			expect(isoOf(text, RUN_14, US)).toBe("2026-09-18");
		for (const text of ["9/18", "26/9/18", "2026/9/18", "20260918"])
			expect(isoOf(text, RUN_14, JA)).toBe("2026-09-18");
	});

	test("ja, ko and zh units", () => {
		expect(isoOf("9月18日", RUN_14, JA)).toBe("2026-09-18");
		expect(isoOf("2026年9月18日", RUN_14, ZH)).toBe("2026-09-18");
		expect(isoOf("2026년 9월 18일", RUN_14, KO)).toBe("2026-09-18");
	});

	test("month names in either position, in the viewer's language", () => {
		expect(isoOf("18. Sept", RUN_14, DE)).toBe("2026-09-18");
		expect(isoOf("18 ago", RUN_14, ES)).toBe("2026-08-18");
		expect(isoOf("18 września 2026", RUN_14, PL)).toBe("2026-09-18");
		expect(isoOf("18 Juli", RUN_14, DE)).toBe("2026-07-18");
	});

	test("a word that fits two months is not read", () => {
		expect(read("18 jui", RUN_14, FR)).toBeNull();
		expect(read("ju 18", RUN_14, GB)).toBeNull();
	});

	test("what cannot be read gives null", () => {
		for (const text of [
			"abc",
			"18/13",
			"31/9/2026",
			"123/9/18",
			"2026/9/2026",
			"18/9/2026/1",
			"18 foo",
		])
			expect(read(text)).toBeNull();
	});

	test("without an anchor the day alone reads against today", () => {
		expect(read("18", null)).toEqual({ iso: "2026-09-18", past: 0 });
		expect(read("1", null)).toEqual({ iso: "2026-09-01", past: 29 });
	});

	test("an unreadable anchor falls back to today; an unreadable today reads nothing", () => {
		expect(isoOf("18", "not a date")).toBe("2026-09-18");
		expect(read("18", RUN_14, GB, "someday")).toBeNull();
	});
});

describe("isoParts, shiftIso and dateExample", () => {
	test("isoParts takes real calendar days only", () => {
		expect(isoParts("2026-02-28")).toEqual({ y: 2026, m: 2, d: 28 });
		expect(isoParts("2024-02-29")).toEqual({ y: 2024, m: 2, d: 29 });
		expect(isoParts("2026-02-29")).toBeNull();
		expect(isoParts("2026-13-01")).toBeNull();
		expect(isoParts("26-09-18")).toBeNull();
		expect(isoParts("")).toBeNull();
	});

	test("shiftIso moves across months and years", () => {
		expect(shiftIso("2026-12-31", 1)).toBe("2027-01-01");
		expect(shiftIso("2026-03-01", -1)).toBe("2026-02-28");
		expect(shiftIso("2026-09-30", 0)).toBe("2026-09-30");
		expect(shiftIso("nope", 1)).toBeNull();
	});

	test("examples in the viewer's order and separator", () => {
		expect(dateExample(GB)).toEqual({ short: "21/9", full: "21/9/2026" });
		expect(dateExample(DE)).toEqual({ short: "21.9", full: "21.9.2026" });
		expect(dateExample(US)).toEqual({ short: "9/21", full: "9/21/2026" });
		expect(dateExample(JA)).toEqual({ short: "9/21", full: "2026/9/21" });
	});
});

function field(
	name: string,
	kind: FieldKind,
	extra: Partial<WorkbenchField> = {},
) {
	const made: WorkbenchField = {
		key: name,
		name,
		label: name,
		help: null,
		kind,
		dataType: "Date",
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
		short: true,
		index: 0,
		...extra,
	};
	return made;
}

const HOST: HostCapabilities = {
	kind: "app",
	presentation: "page",
	persistence: "device",
	memoryScope: "profile:test",
	uploads: "temporary",
	flowPathFiles: true,
	nextFiles: true,
	stop: "cancel",
	inlineFileLimitBytes: null,
	inlineRoomBytes: null,
	warnFileBytes: null,
	fixedTarget: null,
	hasToolbar: false,
};

function run(n: number, values: Record<string, CopyValue>) {
	const made: RunEntry = {
		id: `run-${n}`,
		n,
		origin: "history",
		status: "done",
		createdAt: n * 1000,
		startedAt: n * 1000,
		endedAt: n * 1000 + 1,
		copy: {
			values,
			presetName: null,
			leftAsIs: false,
			perRun: [],
			replaced: [],
		},
		target: null,
		streamId: null,
		backendRunId: null,
		output: null,
		outcome: null,
		summary: {
			firstLine: null,
			stepCount: 0,
			stepReached: null,
			fileCount: 0,
			hasAnswer: false,
			hasResult: false,
		},
		stopRequested: false,
		pendingSlotIds: [],
		waitingForPlace: false,
		failedAt: null,
		unseenFailure: false,
	};
	return made;
}

const VIEWER: ViewerHabits = {
	mac: true,
	locale: "en-GB",
	dateLocale: GB,
	decimalSign: ".",
};

const RAIL: RailState = {
	values: {},
	texts: {},
	problems: {},
	pressed: false,
	nextFiles: {},
	leftOut: {},
	dateAnchors: {},
	entryBegan: {},
	activePresetId: null,
	comparedRunId: null,
	tab: "inputs",
	filter: { query: "", chip: "all" },
	lastPressAt: null,
};

const VIEW: ViewState = {
	selectedRunId: null,
	pinnedRunId: null,
	pane: "inputs",
	overlay: null,
	focus: null,
	message: null,
	question: null,
	list: null,
	outputUnseen: false,
	stageFollowsNewest: true,
};

const PREFS: FormPrefs = {
	version: 2,
	perRun: [],
	auto: [],
	introduced: false,
	offerAnswered: false,
	noSave: [],
	forgotten: {},
	nextRunNumber: 1,
	fieldSeenAt: {},
};

function stateOf(
	fields: readonly WorkbenchField[],
	runs: readonly RunEntry[],
	rail: Partial<RailState> = {},
) {
	const made: FormSessionState = {
		form: {
			appId: "app",
			eventId: "event",
			nodeId: "node",
			name: "Extract invoice",
			description: "",
			fields,
			contentKey: "",
			submitLabel: null,
			routes: [],
			eventRoute: null,
			host: HOST,
			viewer: VIEWER,
		},
		layout: null,
		rail: { ...RAIL, ...rail },
		runs,
		view: VIEW,
		memory: { loaded: true, prefs: PREFS, presets: [] },
		queue: {
			target: "local",
			tierLimit: null,
			cap: 3,
			hold: null,
			retryAt: null,
		},
		announcement: null,
		undo: null,
		seq: 0,
	};
	return made;
}

describe("dateAnchorOf", () => {
	const fromKey = `period${FIELD_KEY_SEPARATOR}from`;
	const fields = [
		field("invoice_date", "date"),
		field("period", "group", {
			dataType: "Struct",
			props: [field("from", "date", { key: fromKey })],
		}),
		field("holidays", "chips", { itemKind: "date", valueType: "Array" }),
	];

	test("this session's last date in the field comes first", () => {
		const state = stateOf(fields, [run(15, { invoice_date: "2026-09-21" })], {
			dateAnchors: { invoice_date: "2026-09-22" },
		});
		expect(dateAnchorOf(state, "invoice_date", TODAY)).toBe("2026-09-22");
	});

	test("else the newest run's value on this device, skipping runs without one", () => {
		const runs = [
			run(16, { invoice_date: "" }),
			run(15, { invoice_date: { $hidden: true } }),
			run(14, { invoice_date: RUN_14 }),
			run(13, { invoice_date: "2026-09-10" }),
		];
		const state = stateOf(fields, runs, {
			dateAnchors: { invoice_date: "junk" },
		});
		expect(dateAnchorOf(state, "invoice_date", TODAY)).toBe(RUN_14);
	});

	test("object properties and lists of dates are read too", () => {
		const runs = [
			run(2, {
				period: { from: "2026-08-01" },
				holidays: ["2026-10-03", "2026-12-25"],
			}),
		];
		const state = stateOf(fields, runs);
		expect(dateAnchorOf(state, fromKey, TODAY)).toBe("2026-08-01");
		expect(dateAnchorOf(state, "holidays", TODAY)).toBe("2026-12-25");
	});

	test("else today, also for a key the form does not have", () => {
		const state = stateOf(fields, []);
		expect(dateAnchorOf(state, "invoice_date", TODAY)).toBe(TODAY);
		expect(dateAnchorOf(state, "gone", TODAY)).toBe(TODAY);
	});
});
