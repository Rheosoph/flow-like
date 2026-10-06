import { describe, expect, test } from "bun:test";
import {
	type CopyValue,
	FIELD_KEY_SEPARATOR,
	type FieldKind,
	type FormPrefs,
	type FormSessionState,
	type HostCapabilities,
	type RailState,
	type RecentSourceEntry,
	type RunEntry,
	type ViewState,
	type ViewerHabits,
	type WorkbenchField,
} from "../contracts";
import { dateLocaleOf } from "./dates";
import { completion, recallFor, recentValues } from "./recent";
import { hashValue } from "./secrets";

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
		...extra,
	};
	return made;
}

const VENDOR = field("vendor_name", "text", {
	label: "Vendor",
	required: true,
});
const COST_CENTERS = field("cost_centers", "chips", {
	label: "Cost centers",
	valueType: "Array",
	itemKind: "text",
	defaultValue: ["4400", "4410"],
	hasDefault: true,
});

const NORDWIND = "Nordwind Logistik GmbH";
const ALPENFRACHT = "Alpenfracht AG";

/** Wed 30 Sep 2026 is "Today" in the spec's history (FLP_HISTORY); minutes since midnight per run. */
const at = (day: number, hour: number, minute: number) =>
	Date.UTC(2026, 8, day, hour, minute);

/** FLP_HISTORY's 14 runs, newest first: Vendor and Cost centers as each run had them. */
const HISTORY: readonly RecentSourceEntry[] = [
	[14, at(30, 14, 2), NORDWIND, ["4400", "4410"]],
	[13, at(30, 13, 58), NORDWIND, ["4400", "4410"]],
	[12, at(30, 11, 20), NORDWIND, ["4400", "4410"]],
	[11, at(30, 9, 47), ALPENFRACHT, ["4400", "4410"]],
	[10, at(29, 17, 31), NORDWIND, ["4400", "4410"]],
	[9, at(29, 16, 5), NORDWIND, ["4400", "4410"]],
	[8, at(29, 15, 40), NORDWIND, ["4400", "4410"]],
	[7, at(29, 15, 22), NORDWIND, ["4400"]],
	[6, at(29, 10, 14), NORDWIND, ["4400"]],
	[5, at(29, 10, 2), NORDWIND, ["4400"]],
	[4, at(28, 16, 48), NORDWIND, ["4400"]],
	[3, at(28, 16, 41), NORDWIND, ["4400"]],
	[2, at(28, 9, 15), NORDWIND, ["4400"]],
	[1, at(28, 9, 3), NORDWIND, ["4400"]],
].map(([, when, vendor, centers]) => ({
	at: when as number,
	values: {
		vendor_name: vendor as string,
		cost_centers: centers as string[],
	},
}));

const entry = (when: number, values: Record<string, CopyValue>) => ({
	at: when,
	values,
});

describe("recentValues", () => {
	test("Vendor at its defaults offers exactly FLP_RECENT.vendor_name", () => {
		expect(recentValues(HISTORY, VENDOR)).toEqual([
			{ value: NORDWIND, at: at(30, 14, 2) },
			{ value: ALPENFRACHT, at: at(30, 9, 47) },
		]);
	});

	test("Cost centers offers nothing while 4400 and 4410 are in, and 4410 once it is removed", () => {
		expect(
			recentValues(HISTORY, COST_CENTERS, { exclude: ["4400", "4410"] }),
		).toEqual([]);
		expect(recentValues(HISTORY, COST_CENTERS, { exclude: ["4400"] })).toEqual([
			{ value: "4410", at: at(30, 14, 2) },
		]);
	});

	test("the typed text filters by prefix or word start, ignoring case and accents", () => {
		expect(recentValues(HISTORY, VENDOR, { query: "log" })).toEqual([
			{ value: NORDWIND, at: at(30, 14, 2) },
		]);
		expect(recentValues(HISTORY, VENDOR, { query: "ALP" })).toEqual([
			{ value: ALPENFRACHT, at: at(30, 9, 47) },
		]);
		expect(recentValues(HISTORY, VENDOR, { query: "ogistik" })).toEqual([]);
		const accented = [entry(1, { vendor_name: "Société Générale" })];
		expect(
			recentValues(accented, VENDOR, { query: "generale" }).map((r) => r.value),
		).toEqual(["Société Générale"]);
	});

	test("values deleted from the list are not offered again", () => {
		expect(
			recentValues(HISTORY, VENDOR, { forgotten: [hashValue(NORDWIND)] }),
		).toEqual([{ value: ALPENFRACHT, at: at(30, 9, 47) }]);
	});

	test("the field's default is left out", () => {
		const withDefault = field("vendor_name", "text", {
			defaultValue: ALPENFRACHT,
			hasDefault: true,
		});
		expect(recentValues(HISTORY, withDefault).map((r) => r.value)).toEqual([
			NORDWIND,
		]);
	});

	test("hidden marks, key-like values, long and multi-line values are left out", () => {
		const source = [
			entry(5, { vendor_name: { $hidden: true } }),
			entry(4, { vendor_name: "sk-live-1234" }),
			entry(3, { vendor_name: "x".repeat(201) }),
			entry(2, { vendor_name: "Line one\nLine two" }),
			entry(1, { vendor_name: "  Kept  " }),
		];
		expect(recentValues(source, VENDOR)).toEqual([{ value: "Kept", at: 1 }]);
	});

	test("secret fields offer nothing, guessed or flagged", () => {
		const source = [entry(1, { api_key: "plain", vendor_name: "plain" })];
		expect(recentValues(source, field("api_key", "text"))).toEqual([]);
		expect(
			recentValues(source, field("vendor_name", "text", { sensitive: true })),
		).toEqual([]);
	});

	test("numbers, dates, switches, choices, files and lists of numbers have no recent values", () => {
		const source = [entry(1, { amount: "12", flag: "x", tags: ["1"] })];
		for (const kind of [
			"number",
			"date",
			"bool",
			"choice",
			"file",
			"json",
		] as const)
			expect(recentValues(source, field("amount", kind))).toEqual([]);
		const numbers = field("tags", "chips", { itemKind: "number" });
		expect(recentValues(source, numbers)).toEqual([]);
	});

	test("at most six by default, each once, from the newest 50 runs only", () => {
		const many = Array.from({ length: 60 }, (_, index) =>
			entry(1000 - index, { vendor_name: `Vendor ${index % 8}` }),
		);
		expect(recentValues(many, VENDOR).map((r) => r.value)).toEqual([
			"Vendor 0",
			"Vendor 1",
			"Vendor 2",
			"Vendor 3",
			"Vendor 4",
			"Vendor 5",
		]);
		expect(recentValues(many, VENDOR, { max: 2 })).toHaveLength(2);
		expect(recentValues(many, VENDOR, { max: 0 })).toEqual([]);
		const old = [
			...Array.from({ length: 50 }, (_, index) =>
				entry(100 - index, { vendor_name: "Recent" }),
			),
			entry(1, { vendor_name: "Too old" }),
		];
		expect(recentValues(old, VENDOR).map((r) => r.value)).toEqual(["Recent"]);
	});

	test("text properties of objects read inside their object", () => {
		const city = field("city", "text", {
			key: `address${FIELD_KEY_SEPARATOR}city`,
		});
		const source = [
			entry(2, { address: { city: "Berlin" }, city: "Not this one" }),
			entry(1, { address: { $hidden: true } }),
		];
		expect(recentValues(source, city)).toEqual([{ value: "Berlin", at: 2 }]);
	});
});

describe("completion", () => {
	test("the newest recent value that starts with the typed text and is longer", () => {
		expect(completion(HISTORY, VENDOR, "N", [])).toBe(NORDWIND);
		expect(completion(HISTORY, VENDOR, "n", [])).toBe(NORDWIND);
		expect(completion(HISTORY, VENDOR, "Alp", [])).toBe(ALPENFRACHT);
	});

	test("word starts belong to the list only; nothing typed, nothing suggested", () => {
		expect(completion(HISTORY, VENDOR, "log", [])).toBeNull();
		expect(completion(HISTORY, VENDOR, "", [])).toBeNull();
		expect(completion(HISTORY, VENDOR, "   ", [])).toBeNull();
		expect(completion(HISTORY, VENDOR, NORDWIND, [])).toBeNull();
	});

	test("typing a shorter value keeps the longer one as the suggestion only", () => {
		const source = [
			entry(1, { vendor_name: "Supplier portal check, September" }),
		];
		expect(completion(source, VENDOR, "Supplier portal check", [])).toBe(
			"Supplier portal check, September",
		);
	});

	test("forgotten values and chips already in the field are not suggested", () => {
		expect(completion(HISTORY, VENDOR, "N", [hashValue(NORDWIND)])).toBeNull();
		expect(
			completion(HISTORY, COST_CENTERS, "44", [], ["4400", "4410"]),
		).toBeNull();
		expect(completion(HISTORY, COST_CENTERS, "44", [], ["4400"])).toBe("4410");
	});
});

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
		origin: "session",
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
		target: "local",
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
	dateLocale: dateLocaleOf("en-GB"),
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
	prefs: Partial<FormPrefs> = {},
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
		rail: RAIL,
		runs,
		view: VIEW,
		memory: { loaded: true, prefs: { ...PREFS, ...prefs }, presets: [] },
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

describe("recallFor", () => {
	const user = field("user", "text", {
		key: `credentials${FIELD_KEY_SEPARATOR}user`,
	});
	const city = field("city", "text", {
		key: `address${FIELD_KEY_SEPARATOR}city`,
	});
	const fields = [
		VENDOR,
		field("max_pages", "number"),
		field("credentials", "group", { props: [user] }),
		field("address", "group", { props: [city] }),
	];
	const runs = [
		run(2, { vendor_name: NORDWIND }),
		run(1, { vendor_name: ALPENFRACHT }),
	];

	test("a text field with a fine pointer reads the runs, newest first, with its forgotten values", () => {
		const forgotten = { vendor_name: [hashValue(ALPENFRACHT)] };
		const recall = recallFor(
			stateOf(fields, runs, { forgotten }),
			VENDOR,
			true,
		);
		expect(recall?.forgotten).toEqual([hashValue(ALPENFRACHT)]);
		expect(recall?.source).toEqual([
			{ at: 2000, values: { vendor_name: NORDWIND } },
			{ at: 1000, values: { vendor_name: ALPENFRACHT } },
		]);
		expect(recentValues(recall?.source ?? [], VENDOR, recall ?? {})).toEqual([
			{ value: NORDWIND, at: 2000 },
		]);
	});

	test("every field of one state shares one source", () => {
		const state = stateOf(fields, runs);
		expect(recallFor(state, VENDOR, true)?.source).toBe(
			recallFor(state, city, true)?.source as RecentSourceEntry[],
		);
	});

	test('none for a coarse pointer, other kinds, secrets, "Don\'t save" and properties of a secret object', () => {
		const state = stateOf(fields, runs);
		expect(recallFor(state, VENDOR, false)).toBeNull();
		expect(recallFor(state, field("max_pages", "number"), true)).toBeNull();
		expect(recallFor(state, field("api_key", "text"), true)).toBeNull();
		expect(
			recallFor(
				stateOf(fields, runs, { noSave: ["vendor_name"] }),
				VENDOR,
				true,
			),
		).toBeNull();
		expect(recallFor(state, user, true)).toBeNull();
		expect(recallFor(state, city, true)).not.toBeNull();
	});
});
