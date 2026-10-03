/*
 * Person-started runs on a fake agent (design R2 §1.8, §4.2–§4.4): what
 * `run_event` starts, what `operation` reads of it, `cancel_run`, and the
 * `event_form` answer. A run moves on with the reads of its operation, so a
 * test decides how and when it ends:
 *
 *   agent.runs.script = { queuedReads: 1, runningReads: 2, end: { output: { id: 42 } } };
 *   agent.runs.script = { runningReads: null };            // runs until …
 *   agent.runs.end(operationId, { code: "flow_failed" });  // … the test ends it
 *   agent.restartAgent();                                  // rows stay, outputs are gone
 *
 * Like the device, the fake never keeps a run's inputs: the field check runs
 * when the run is accepted and only the names it refused are kept.
 */

const encoder = new TextEncoder();
const bytesOf = (value: unknown) =>
	encoder.encode(JSON.stringify(value)).length;

export const RUN_CODES = [
	"flow_failed",
	"invalid_fields",
	"cancelled",
	"timed_out",
	"interrupted",
	"not_started",
	"needs_interaction",
] as const;
export type FakeRunCode = (typeof RUN_CODES)[number];
export type FakeRunState =
	| "queued"
	| "running"
	| "succeeded"
	| "failed"
	| "cancelled"
	| "timed_out";

/** How a run ends. Without a code it succeeded. */
export interface FakeRunEnd {
	code?: FakeRunCode;
	/** With `invalid_fields`: the names the device refused. */
	fields?: readonly string[];
	/** The flow's result (its last `generic_result`). */
	output?: unknown;
	/** A text result (`chat_out`, `text_output`) instead of a value. */
	text?: string;
	/** Media parts of the result, counted and never copied. */
	attachments?: number;
}

export interface FakeRunScript {
	/** Operation reads the run waits for an instance; 0 by default. */
	queuedReads?: number;
	/** Reads it then runs before it ends; 1 by default, null = until `end()`. */
	runningReads?: number | null;
	end?: FakeRunEnd;
}

/** One field of a form as `event_form` sends it (design R2 §1.8). */
export interface FakeFormField {
	name: string;
	label: string;
	description: string;
	data_type: string;
	value_type: string;
	optional: boolean;
	sensitive: boolean;
	default: unknown;
	options: string[] | null;
	default_omitted?: boolean;
}

/** What the flow version a service runs says about its form: the entry node's pins. */
export interface FakeEventForm {
	description?: string;
	fields: readonly FakeFormField[];
	navigate_to_routes?: readonly string[];
}

/** A field with the device's defaults: a required text field. */
export function formField(
	name: string,
	field: Partial<Omit<FakeFormField, "name">> = {},
): FakeFormField {
	return {
		name,
		label: name.replace(/^./, (first) => first.toUpperCase()),
		description: "",
		data_type: "String",
		value_type: "Normal",
		optional: false,
		sensitive: false,
		default: null,
		options: null,
		...field,
	};
}

/**
 * The forms of the sample's person-started events, as their flows define them
 * (the field kinds of the fixtures' `fields`). `evt_notes_form` is the form of
 * design R2 §1.6c and §1.8.
 */
export const SAMPLE_FORMS: Record<string, FakeEventForm> = {
	evt_support_reply: { fields: [] },
	evt_notes_form: { fields: [formField("title")] },
	evt_shop_return: {
		description: "Asks the shop to take an order back.",
		fields: [
			formField("order", { label: "Order number" }),
			formField("quantity", {
				data_type: "Integer",
				label: "Quantity",
				default: 1,
			}),
			formField("receipt", { data_type: "PathBuf", label: "Receipt" }),
		],
	},
};

export const MAX_RUN_OUTPUT_BYTES = 8_192;
export const MAX_RUN_PAYLOAD_BYTES = 12_288;
export const MAX_RUN_PAYLOAD_KEYS = 64;
const MAX_FORM_BYTES = 12 * 1024;
const MAX_FORM_FIELDS = 64;
const MAX_REFUSED_NAMES = 16;
const MAX_REFUSED_NAME = 64;
const MAX_REFUSED_BYTES = 2_048;
const OPEN_RUNS_PER_SERVICE = 4 + 8;
const FILE_TYPES = new Set(["PathBuf", "Byte"]);

/** Why the parent refuses a payload before anything is journaled; null when it fits. */
export function payloadProblem(payload: unknown): string | null {
	if (payload === undefined) return null;
	if (!payload || typeof payload !== "object" || Array.isArray(payload))
		return "The payload is not an object.";
	if (Object.keys(payload).length > MAX_RUN_PAYLOAD_KEYS)
		return `The payload has more than ${MAX_RUN_PAYLOAD_KEYS} fields.`;
	return bytesOf(payload) > MAX_RUN_PAYLOAD_BYTES
		? `The payload is larger than ${MAX_RUN_PAYLOAD_BYTES} bytes.`
		: null;
}

const RFC3339 =
	/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

function accepts(field: FakeFormField, value: unknown): boolean {
	if (field.value_type === "Array" || field.value_type === "HashSet")
		return Array.isArray(value);
	if (field.value_type === "HashMap")
		return !!value && typeof value === "object" && !Array.isArray(value);
	switch (field.data_type) {
		case "String":
		case "Geometry":
			return typeof value === "string";
		case "Integer":
			return Number.isSafeInteger(value);
		case "Float":
			return typeof value === "number" && Number.isFinite(value);
		case "Boolean":
			return typeof value === "boolean";
		case "Date":
			return (
				typeof value === "string" &&
				(/^\d{4}-\d{2}-\d{2}$/.test(value) || RFC3339.test(value))
			);
		case "Struct":
			return !!value && typeof value === "object" && !Array.isArray(value);
		case "Generic":
			return true;
		default:
			return false;
	}
}

/**
 * The device's field check (§4.3): a key that is no field, a required field
 * without a value, a value of the wrong kind, and every file field (a file
 * does not fit `run_event`). Returns the refused names as the device reports them.
 */
export function refusedFields(
	form: FakeEventForm,
	payload: Record<string, unknown> | undefined,
): string[] {
	const values = payload ?? {};
	const known = new Map(form.fields.map((field) => [field.name, field]));
	const refused = [
		...Object.keys(values).filter((name) => !known.has(name)),
		...form.fields
			.filter((field) => {
				const value = values[field.name];
				if (value === undefined || value === null) return !field.optional;
				return FILE_TYPES.has(field.data_type) || !accepts(field, value);
			})
			.map((field) => field.name),
	];
	const names: string[] = [];
	for (const name of refused.slice(0, MAX_REFUSED_NAMES)) {
		const cut = [...name].slice(0, MAX_REFUSED_NAME).join("");
		if (bytesOf([...names, cut]) > MAX_REFUSED_BYTES) break;
		names.push(cut);
	}
	return names;
}

/** Fields that take a file (`PathBuf`, `Byte`): only the service page can send them. */
export const fileFields = (fields: readonly FakeFormField[]) =>
	fields.filter((field) => FILE_TYPES.has(field.data_type)).length;

/** `action` (no fields) or `form`, as the contract file says. */
export const formKind = (form: FakeEventForm) =>
	form.fields.length ? "form" : "action";

/** The fields as the device writes them: a sensitive field's default never leaves it. */
function deviceFields(form: FakeEventForm): FakeFormField[] {
	return form.fields.slice(0, MAX_FORM_FIELDS).map((field) => ({
		...field,
		default: field.sensitive ? null : field.default,
	}));
}

/** The `event_form` answer, at most 12 KiB as it is sent: fields are dropped from the end. */
export function formAnswer(
	head: {
		placement_id: string;
		config_revision: number;
		event_id: string;
		event_version: readonly number[];
		board_version: readonly number[];
		name: string;
	},
	form: FakeEventForm,
): Record<string, unknown> {
	const fields = deviceFields(form);
	const answer = (kept: FakeFormField[]) => ({
		...head,
		kind: formKind(form),
		description: form.description ?? "",
		fields: kept,
		fields_truncated: kept.length < form.fields.length,
		file_fields: fileFields(form.fields),
		navigate_to_routes: [...(form.navigate_to_routes ?? [])],
	});
	let kept = fields;
	while (kept.length && bytesOf(answer(kept)) > MAX_FORM_BYTES)
		kept = kept.slice(0, -1);
	return answer(kept);
}

/** The run's output as it travels: `{json}` when it fits 8 KiB, else text cut until it does. */
export function boundedOutput(end: FakeRunEnd): {
	output: Record<string, unknown> | null;
	output_bytes: number;
	truncated: boolean;
} {
	if (end.output === undefined && end.text === undefined)
		return { output: null, output_bytes: 0, truncated: false };
	const whole =
		end.text !== undefined ? { text: end.text } : { json: end.output };
	const outputBytes =
		end.text !== undefined
			? encoder.encode(end.text).length
			: bytesOf(end.output);
	if (bytesOf(whole) <= MAX_RUN_OUTPUT_BYTES)
		return { output: whole, output_bytes: outputBytes, truncated: false };
	const characters = [
		...(end.text !== undefined ? end.text : JSON.stringify(end.output)),
	];
	let low = 0;
	let high = characters.length;
	while (low < high) {
		const middle = Math.ceil((low + high) / 2);
		if (
			bytesOf({ text: characters.slice(0, middle).join("") }) <=
			MAX_RUN_OUTPUT_BYTES
		)
			low = middle;
		else high = middle - 1;
	}
	return {
		output: { text: characters.slice(0, low).join("") },
		output_bytes: outputBytes,
		truncated: true,
	};
}

const RUN_OF_CODE: Partial<Record<FakeRunCode, FakeRunState>> = {
	cancelled: "cancelled",
	timed_out: "timed_out",
};

interface RunRecord {
	operationId: string;
	placementId: string;
	eventId: string;
	runId: string;
	run: FakeRunState;
	code?: FakeRunCode;
	fields?: string[];
	startedAt?: number;
	finishedAt?: number;
	output?: Record<string, unknown> | null;
	outputBytes: number;
	truncated: boolean;
	attachments: number;
	outputGone: boolean;
	reads: number;
	queuedReads: number;
	runningReads: number | null;
	end: FakeRunEnd;
}

const OPEN: ReadonlySet<FakeRunState> = new Set(["queued", "running"]);

/** One placement's counters of person-started runs, as the run state files add them up. */
export interface FakeRunCounters {
	running: number;
	runs: number;
	failed: number;
	last_at: number | null;
	last_outcome: "succeeded" | "failed" | "cancelled" | "timed_out" | null;
}

/** The person-started runs one agent has in its memory, by operation id. */
export class FakeRuns {
	/** How runs started from now on go. */
	script: FakeRunScript = {};
	private readonly records = new Map<string, RunRecord>();

	constructor(
		private readonly now: () => number,
		/** A run started or ended: the service's row facts move. */
		private readonly changed: (placementId: string) => void = () => {},
	) {}

	get(operationId: string): RunRecord | undefined {
		return this.records.get(operationId);
	}

	/** Runs of a service that are queued or running. */
	open(placementId: string): RunRecord[] {
		return [...this.records.values()].filter(
			(record) => record.placementId === placementId && OPEN.has(record.run),
		);
	}

	/** Four running and eight waiting: the parent answers `busy`. */
	full(placementId: string): boolean {
		return this.open(placementId).length >= OPEN_RUNS_PER_SERVICE;
	}

	/** Accepted: queued with a new run id. `refused` are the names the field check refused. */
	start(
		operationId: string,
		placementId: string,
		eventId: string,
		refused: readonly string[] = [],
	): RunRecord {
		const script = this.script;
		const record: RunRecord = {
			operationId,
			placementId,
			eventId,
			runId: crypto.randomUUID(),
			run: "queued",
			outputBytes: 0,
			truncated: false,
			attachments: 0,
			outputGone: false,
			reads: 0,
			queuedReads: Math.max(0, script.queuedReads ?? 0),
			runningReads:
				script.runningReads === null ? null : (script.runningReads ?? 1),
			end: refused.length
				? { code: "invalid_fields", fields: refused }
				: (script.end ?? {}),
		};
		this.records.set(operationId, record);
		this.changed(placementId);
		return record;
	}

	private begin(record: RunRecord) {
		record.run = "running";
		record.startedAt = this.now();
		this.changed(record.placementId);
	}

	private finish(record: RunRecord, end: FakeRunEnd) {
		const code = end.code;
		record.run = code ? (RUN_OF_CODE[code] ?? "failed") : "succeeded";
		record.code = code;
		record.fields =
			code === "invalid_fields" ? [...(end.fields ?? [])] : undefined;
		if (code === "not_started") record.startedAt = undefined;
		else record.startedAt ??= this.now();
		record.finishedAt = this.now();
		const bounded = code
			? { output: null, output_bytes: 0, truncated: false }
			: boundedOutput(end);
		record.output = bounded.output;
		record.outputBytes = bounded.output_bytes;
		record.truncated = bounded.truncated;
		record.attachments = code ? 0 : (end.attachments ?? 0);
		this.changed(record.placementId);
	}

	/** One `operation` read: the run moves on as its script says. */
	read(operationId: string): RunRecord | undefined {
		const record = this.records.get(operationId);
		if (!record || !OPEN.has(record.run)) return record;
		record.reads += 1;
		if (record.run === "queued" && record.reads > record.queuedReads) {
			if (record.end.code === "not_started") {
				this.finish(record, record.end);
				return record;
			}
			this.begin(record);
		}
		if (
			record.run === "running" &&
			record.runningReads !== null &&
			record.reads > record.queuedReads + record.runningReads
		)
			this.finish(record, record.end);
		return record;
	}

	/** Ends an open run now; false when it had ended already or is unknown. */
	end(operationId: string, end: FakeRunEnd = {}): boolean {
		const record = this.records.get(operationId);
		if (!record || !OPEN.has(record.run)) return false;
		this.finish(record, end);
		return true;
	}

	/** "Stop this run": undefined when the device has no such run. */
	cancel(operationId: string): boolean | undefined {
		if (!this.records.has(operationId)) return undefined;
		return this.end(operationId, { code: "cancelled" });
	}

	/** The service stopped, updated or restarted: a running run is cut off, a waiting one never starts. */
	interrupt(placementId: string): void {
		for (const record of this.open(placementId))
			this.finish(record, {
				code: record.run === "running" ? "interrupted" : "not_started",
			});
	}

	/** The agent restarted: open runs end interrupted, every kept output is gone. */
	restartAgent(): void {
		for (const record of this.records.values()) {
			if (OPEN.has(record.run)) this.finish(record, { code: "interrupted" });
			if (record.output) record.outputGone = true;
			record.output = null;
		}
	}

	/** Top-level state of the operation row. */
	state(record: RunRecord): "accepted" | "completed" | "failed" {
		if (OPEN.has(record.run)) return "accepted";
		return record.run === "succeeded" ? "completed" : "failed";
	}

	/** The row `operation` answers: the stored row, and the output from memory while it is there. */
	result(record: RunRecord): Record<string, unknown> {
		const ended = !OPEN.has(record.run);
		return {
			command: "run_event",
			placement_id: record.placementId,
			event_id: record.eventId,
			run_id: record.runId,
			run: record.run,
			...(record.code ? { code: record.code } : {}),
			...(record.fields ? { fields: record.fields } : {}),
			...(record.startedAt === undefined
				? {}
				: { started_at: record.startedAt }),
			...(ended
				? {
						finished_at: record.finishedAt,
						output_bytes: record.outputBytes,
						truncated: record.truncated,
						attachments: record.attachments,
						...(record.output ? { output: record.output } : {}),
						...(record.outputGone ? { output_gone: true } : {}),
					}
				: {}),
		};
	}

	/** The service's counters of one event (the `actions` row fact). */
	counters(placementId: string, eventId: string): FakeRunCounters {
		const records = [...this.records.values()].filter(
			(record) =>
				record.placementId === placementId && record.eventId === eventId,
		);
		const ended = records.filter((record) => !OPEN.has(record.run));
		const last = ended.reduce<RunRecord | undefined>(
			(newest, record) =>
				!newest || (record.finishedAt ?? 0) >= (newest.finishedAt ?? 0)
					? record
					: newest,
			undefined,
		);
		return {
			running: records.filter((record) => record.run === "running").length,
			runs: ended.filter((record) => record.code !== "not_started").length,
			failed: ended.filter(
				(record) => record.run === "failed" || record.run === "timed_out",
			).length,
			last_at: last?.finishedAt ?? null,
			last_outcome: last
				? last.run === "succeeded" ||
					last.run === "cancelled" ||
					last.run === "timed_out"
					? last.run
					: "failed"
				: null,
		};
	}
}
