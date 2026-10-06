import { describe, expect, test } from "bun:test";
import type {
	DockLine,
	DockLineInput,
	DockLineOf,
	DockMessageEntry,
	DockQuestion,
	FieldKind,
	FieldValues,
	FileSlot,
	FileSlotState,
	FormModel,
	FormPrefs,
	FormSessionState,
	HoldInfo,
	HostCapabilities,
	RailState,
	RunEntry,
	RunStatus,
	StepRef,
	ViewState,
	WorkbenchField,
} from "../contracts";
import { dateLocaleOf } from "./dates";
import { dockLine, dockLineInputOf, queueSummary } from "./dock";

const _contract: DockLineOf = dockLine;

const OCR: StepRef = { number: 2, title: "Run OCR" };
const HOLD: HoldInfo = { runs: [18, 19], step: OCR };
const MESSAGE: DockMessageEntry = {
	seq: 1,
	message: { kind: "fieldReset", label: "Max pages" },
	undo: true,
	expiresAt: null,
};
const QUESTION: DockQuestion = { kind: "offer", names: ["order", "receipt"] };

const QUIET: DockLineInput = {
	sending: null,
	problems: 0,
	hold: null,
	message: null,
	failures: [],
	question: null,
	running: 0,
	queued: 0,
	blocked: [],
	compared: null,
	missing: 0,
};

const EVERYTHING: DockLineInput = {
	sending: { left: 2, total: 3 },
	problems: 2,
	hold: HOLD,
	message: MESSAGE,
	failures: [{ runId: "run-18", n: 18, step: OCR }],
	question: QUESTION,
	running: 3,
	queued: 1,
	blocked: ["Invoice"],
	compared: { n: 17, changes: 1 },
	missing: 2,
};

describe("dockLine", () => {
	test("one thing at a time, highest first", () => {
		const steps: [Partial<DockLineInput>, DockLine["kind"]][] = [
			[{}, "sending"],
			[{ sending: null }, "problems"],
			[{ problems: 0 }, "hold"],
			[{ hold: null }, "message"],
			[{ message: null }, "failure"],
			[{ failures: [] }, "question"],
			[{ question: null }, "queue"],
			[{ running: 0, queued: 0 }, "blocked"],
			[{ blocked: [] }, "compared"],
			[{ compared: null }, "missing"],
			[{ missing: 0 }, "ready"],
		];
		let input = EVERYTHING;
		for (const [change, kind] of steps) {
			input = { ...input, ...change };
			expect(dockLine(input).kind).toBe(kind);
		}
	});

	test("Sending 2 of 3 files, Sending 1 file", () => {
		expect(dockLine({ ...QUIET, sending: { left: 2, total: 3 } })).toEqual({
			kind: "sending",
			current: 2,
			total: 3,
		});
		expect(dockLine({ ...QUIET, sending: { left: 1, total: 1 } })).toEqual({
			kind: "sending",
			current: 1,
			total: 1,
		});
		expect(dockLine({ ...QUIET, sending: { left: 3, total: 1 } })).toEqual({
			kind: "sending",
			current: 1,
			total: 3,
		});
		expect(dockLine({ ...QUIET, sending: { left: 0, total: 2 } })).toEqual({
			kind: "ready",
		});
	});

	test("the queue line shows when 2 or more run or any wait", () => {
		expect(dockLine({ ...QUIET, running: 1 }).kind).toBe("ready");
		expect(dockLine({ ...QUIET, running: 2 })).toEqual({
			kind: "queue",
			running: 2,
			queued: 0,
		});
		expect(dockLine({ ...QUIET, running: 3, queued: 1 })).toEqual({
			kind: "queue",
			running: 3,
			queued: 1,
		});
		expect(dockLine({ ...QUIET, queued: 1 }).kind).toBe("queue");
	});

	test("each line carries what its words need", () => {
		expect(dockLine({ ...QUIET, problems: 2 })).toEqual({
			kind: "problems",
			count: 2,
		});
		expect(dockLine({ ...QUIET, hold: HOLD })).toEqual({
			kind: "hold",
			hold: HOLD,
		});
		expect(dockLine({ ...QUIET, message: MESSAGE })).toEqual({
			kind: "message",
			entry: MESSAGE,
		});
		expect(dockLine({ ...QUIET, failures: EVERYTHING.failures })).toEqual({
			kind: "failure",
			runs: EVERYTHING.failures,
		});
		expect(dockLine({ ...QUIET, question: QUESTION })).toEqual({
			kind: "question",
			question: QUESTION,
		});
		expect(dockLine({ ...QUIET, blocked: ["Invoice", "Receipt"] })).toEqual({
			kind: "blocked",
			label: "Invoice",
		});
		expect(dockLine({ ...QUIET, compared: { n: 3, changes: 0 } })).toEqual({
			kind: "compared",
			n: 3,
			changes: 0,
		});
		expect(dockLine({ ...QUIET, missing: 3 })).toEqual({
			kind: "missing",
			count: 3,
		});
		expect(dockLine(QUIET)).toEqual({ kind: "ready" });
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

function slot(id: string, state: FileSlotState, sentAt: number | null = null) {
	const made: FileSlot = {
		id,
		name: `${id}.pdf`,
		size: 1000,
		type: "application/pdf",
		state,
		progress: state === "sending" ? 0.4 : null,
		ref: null,
		error: null,
		sentAt,
		expiresAt: null,
	};
	return made;
}

function run(n: number, status: RunStatus, extra: Partial<RunEntry> = {}) {
	const made: RunEntry = {
		id: `run-${n}`,
		n,
		origin: "session",
		status,
		createdAt: n * 1000,
		startedAt: n * 1000,
		endedAt: null,
		copy: {
			values: {},
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

const FIELDS = [
	field("invoice_file", "file", {
		label: "Invoice",
		required: true,
		fileMode: "flowpath",
	}),
	field("supporting_documents", "files", {
		label: "Supporting documents",
		fileMode: "flowpath",
		valueType: "Array",
	}),
	field("vendor_name", "text", { label: "Vendor", required: true }),
];

const FORM: FormModel = {
	appId: "app",
	eventId: "event",
	nodeId: "node",
	name: "Extract invoice",
	description: "",
	fields: FIELDS,
	contentKey: "",
	submitLabel: null,
	routes: [],
	eventRoute: null,
	host: HOST,
	viewer: {
		mac: true,
		locale: "en-GB",
		dateLocale: dateLocaleOf("en-GB"),
		decimalSign: ".",
	},
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

interface Parts {
	readonly fields: readonly WorkbenchField[];
	readonly values: FieldValues;
	readonly rail: Partial<RailState>;
	readonly runs: readonly RunEntry[];
	readonly hold: HoldInfo | null;
	readonly message: DockMessageEntry | null;
	readonly question: DockQuestion | null;
}

const NO_PARTS: Parts = {
	fields: FIELDS,
	values: {},
	rail: {},
	runs: [],
	hold: null,
	message: null,
	question: null,
};

function stateOf(given: Partial<Parts> = {}) {
	const parts = { ...NO_PARTS, ...given };
	const made: FormSessionState = {
		form: { ...FORM, fields: parts.fields },
		layout: null,
		rail: { ...RAIL, values: parts.values, ...parts.rail },
		runs: parts.runs,
		view: { ...VIEW, message: parts.message, question: parts.question },
		memory: { loaded: true, prefs: PREFS, presets: [] },
		queue: {
			target: "local",
			tierLimit: null,
			cap: 3,
			hold: parts.hold,
			retryAt: null,
		},
		announcement: null,
		undo: null,
		seq: 0,
	};
	return made;
}

const NOTHING_DERIVED = { comparedChanges: null, missing: 0, blocked: [] };
const lineOf = (
	state: FormSessionState,
	derived: Parameters<typeof dockLineInputOf>[1] = NOTHING_DERIVED,
) => dockLine(dockLineInputOf(state, derived));

describe("dockLineInputOf", () => {
	test("uploading: the sent invoice and two documents still sending read Sending 2 of 3 files", () => {
		const state = stateOf({
			values: {
				invoice_file: slot("invoice", "sent", 100),
				supporting_documents: [
					slot("doc-1", "sending"),
					slot("doc-2", "waiting"),
				],
			},
		});
		expect(dockLineInputOf(state, NOTHING_DERIVED).sending).toEqual({
			left: 2,
			total: 3,
		});
		expect(lineOf(state)).toEqual({ kind: "sending", current: 2, total: 3 });
	});

	test("after a press, files sent before it no longer count; next files never do", () => {
		const state = stateOf({
			values: {
				invoice_file: slot("invoice", "sending"),
				supporting_documents: [
					slot("doc-1", "sent", 100),
					slot("doc-2", "sent", 300),
				],
			},
			rail: {
				lastPressAt: 200,
				nextFiles: {
					invoice_file: [slot("next-1", "sending"), slot("next-2", "waiting")],
				},
			},
		});
		expect(dockLineInputOf(state, NOTHING_DERIVED).sending).toEqual({
			left: 1,
			total: 2,
		});
	});

	test("next-files: only the current invoice is this entry's file, Sending 1 file", () => {
		const state = stateOf({
			values: {
				invoice_file: slot("0918", "sending"),
				supporting_documents: [],
			},
			rail: { nextFiles: { invoice_file: [slot("0919", "sending")] } },
		});
		expect(
			lineOf(state, { comparedChanges: null, missing: 2, blocked: [] }),
		).toEqual({
			kind: "sending",
			current: 1,
			total: 1,
		});
	});

	test("failed files and Pick again reminders are not sending", () => {
		const state = stateOf({
			values: {
				invoice_file: slot("invoice", "failed"),
				supporting_documents: [slot("old", "reminder")],
			},
		});
		expect(dockLineInputOf(state, NOTHING_DERIVED).sending).toBeNull();
	});

	test("problems count only after a press", () => {
		const problems = {
			invoice_file: { code: "required" as const },
			vendor_name: { code: "required" as const },
		};
		expect(
			dockLineInputOf(stateOf({ rail: { problems } }), NOTHING_DERIVED)
				.problems,
		).toBe(0);
		const pressed = stateOf({ rail: { problems, pressed: true } });
		expect(lineOf(pressed)).toEqual({ kind: "problems", count: 2 });
	});

	test("queued: 3 running · 1 queued", () => {
		const state = stateOf({
			runs: [
				run(18, "queued"),
				run(17, "running"),
				run(16, "running"),
				run(15, "running"),
				run(14, "done", { origin: "history" }),
			],
		});
		expect(lineOf(state)).toEqual({ kind: "queue", running: 3, queued: 1 });
	});

	test("series-failed: an unseen failure outranks the queue line", () => {
		const state = stateOf({
			runs: [
				run(23, "queued"),
				run(22, "queued"),
				run(21, "running"),
				run(20, "running"),
				run(19, "running"),
				run(18, "failed", { unseenFailure: true, failedAt: OCR }),
				run(17, "done"),
			],
		});
		expect(lineOf(state)).toEqual({
			kind: "failure",
			runs: [{ runId: "run-18", n: 18, step: OCR }],
		});
	});

	test("series: the start message outranks the queue line; the hold outranks both", () => {
		const runs = [run(18, "queued"), run(17, "running"), run(16, "running")];
		const start: DockMessageEntry = {
			seq: 4,
			message: {
				kind: "start",
				start: {
					n: 18,
					state: "queued",
					files: 1,
					pairs: ["invoice-RE-2026-0921.pdf", "22 Sep 2026"],
					lastFile: false,
					leftAsIs: false,
				},
			},
			undo: false,
			expiresAt: 4000,
		};
		expect(lineOf(stateOf({ runs, message: start }))).toEqual({
			kind: "message",
			entry: start,
		});
		expect(lineOf(stateOf({ runs, message: start, hold: HOLD }))).toEqual({
			kind: "hold",
			hold: HOLD,
		});
	});

	test("small-offer: the offer question", () => {
		const state = stateOf({ runs: [run(3, "running")], question: QUESTION });
		expect(lineOf(state)).toEqual({ kind: "question", question: QUESTION });
	});

	test("hosted-files: a required file field this page cannot fill, by its label", () => {
		const blocked = ["invoice_file", "supporting_documents"];
		expect(
			dockLineInputOf(stateOf(), { ...NOTHING_DERIVED, blocked }).blocked,
		).toEqual(["Invoice"]);
		expect(
			lineOf(stateOf(), { comparedChanges: null, missing: 2, blocked }),
		).toEqual({
			kind: "blocked",
			label: "Invoice",
		});
	});

	test("the comparison names the compared run; Same inputs as run 3 has no changes", () => {
		const runs = [run(17, "done"), run(3, "done")];
		const state = stateOf({ runs, rail: { comparedRunId: "run-17" } });
		expect(
			lineOf(state, { comparedChanges: 1, missing: 0, blocked: [] }),
		).toEqual({
			kind: "compared",
			n: 17,
			changes: 1,
		});
		const same = stateOf({ runs, rail: { comparedRunId: "run-3" } });
		expect(
			lineOf(same, { comparedChanges: 0, missing: 0, blocked: [] }),
		).toEqual({
			kind: "compared",
			n: 3,
			changes: 0,
		});
	});

	test("no comparison without a compared run, a count, the run itself or any field", () => {
		const runs = [run(17, "done")];
		const derived = { comparedChanges: 1, missing: 0, blocked: [] };
		expect(dockLineInputOf(stateOf({ runs }), derived).compared).toBeNull();
		const compared = { comparedRunId: "run-17" };
		expect(
			dockLineInputOf(stateOf({ runs, rail: compared }), NOTHING_DERIVED)
				.compared,
		).toBeNull();
		const gone = { comparedRunId: "run-99" };
		expect(
			dockLineInputOf(stateOf({ runs, rail: gone }), derived).compared,
		).toBeNull();
		const noFields = stateOf({ fields: [], runs, rail: compared });
		expect(dockLineInputOf(noFields, derived).compared).toBeNull();
	});

	test("reopen: 3 fields to fill in; then Ready", () => {
		expect(
			lineOf(stateOf(), { comparedChanges: null, missing: 3, blocked: [] }),
		).toEqual({
			kind: "missing",
			count: 3,
		});
		expect(lineOf(stateOf())).toEqual({ kind: "ready" });
	});
});

describe("queueSummary", () => {
	test("runs holding a place, queued, sending, and the hold", () => {
		const runs = [
			run(8, "sending"),
			run(7, "queued"),
			run(6, "queued"),
			run(5, "streaming"),
			run(4, "asking"),
			run(3, "running"),
			run(2, "starting"),
			run(1, "done"),
		];
		expect(queueSummary(stateOf({ runs }))).toEqual({
			busy: 4,
			queued: 2,
			sending: 1,
			held: false,
		});
		expect(queueSummary(stateOf({ hold: HOLD })).held).toBe(true);
	});
});
