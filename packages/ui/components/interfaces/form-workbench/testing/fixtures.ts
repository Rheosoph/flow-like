import type { IInteractionRequest } from "../../../../lib/schema/interaction";
import type { IAttachment } from "../../chat-default/chat-db";
import {
	type Announcement,
	type CopyValue,
	type DateLocale,
	type DockMessage,
	type DockMessageEntry,
	FIELD_KEY_SEPARATOR,
	FORM_LIMITS,
	type FieldKey,
	type FieldKind,
	type FieldValue,
	type FieldValues,
	type FileMode,
	type FileSlot,
	type FlowPath,
	type FocusRequest,
	type FormHostKind,
	type FormModel,
	type FormPrefs,
	type FormSessionState,
	type HostCapabilities,
	type Overlay,
	type Preset,
	type QueueState,
	REQUEST_FILES_STORE_REF,
	type RailState,
	type RunEntry,
	type RunOutcome,
	type RunOutput,
	type RunStatus,
	type RunStep,
	type RunStepState,
	type RunSummary,
	type SessionClock,
	type StepRef,
	type TerminalSignals,
	type UndoEntry,
	type ViewState,
	type ViewerHabits,
	type WorkbenchField,
} from "../contracts";
import { type ArtboardDevice, DESKTOP_LAYOUT } from "./layouts";

/*
 * Hand-written session states for every artboard of the canvas and every preset of spec §8
 * (PLAN §12.6). They are data, not computed by the model, so UI lanes can render them before the
 * session exists and S-STATE can start its tests from them.
 *
 * World: Main's forms (FLP_FORMS: FlowPath pins where DATA.js has PathBuf), Main's viewer (macOS,
 * en-GB date habits, decimal "."), the desktop app host (profile:local, runs on this device, 3 at a
 * time) except `hosted-files`. Today is Mon 5 Oct 2026; times are local wall-clock times, so
 * "Today 14:02" reads the same in every time zone. `fixtureClock(name)` is the moment each state
 * shows; the harness freezes the page clock there. Runs are newest first, this session's first.
 */

export const FIXTURE_NAMES = [
	"idle",
	"invalid",
	"running",
	"streaming",
	"done",
	"failed",
	"stopped",
	"none",
	"none-done",
	"small",
	"small-done",
	"large",
	"compare",
	"runs",
	"uploading",
	"reopen",
	"next-files",
	"recent",
	"series",
	"series-failed",
	"queued",
	"after-run",
	"small-offer",
	"small-reset",
	"none-cap",
	"hosted-files",
	"shortcuts",
	"presets",
	"preset-save",
	"pick-again",
	"asking",
	"leave",
] as const;

export type FixtureName = (typeof FIXTURE_NAMES)[number];

/** The four sample forms: zero fields, three, nine, twenty-nine. */
export type FormKey = "none" | "small" | "medium" | "large";

// ─── Time ───────────────────────────────────────────────────────────────────

export const FIXTURE_TODAY = "2026-10-05";

type DayKey = "today" | "yesterday" | "mon28";

const DAYS: Readonly<Record<DayKey, readonly [number, number, number]>> = {
	today: [2026, 9, 5],
	yesterday: [2026, 9, 4],
	mon28: [2026, 8, 28],
};

const SECOND = 1000;

/** A local wall-clock moment: `at("today", "14:02:30")`. */
function at(day: DayKey, time: string): number {
	const [year, month, date] = DAYS[day];
	const [hours = 0, minutes = 0, seconds = 0] = time.split(":").map(Number);
	return new Date(year, month, date, hours, minutes, seconds).getTime();
}

const today = (time: string) => at("today", time);

/** Run 14 of Extract invoice, the run the base artboards show. */
const RUN14 = { created: today("14:02:00"), started: today("14:02:01") };
/** Run 13, the failed one. */
const RUN13 = { created: today("13:58:00"), started: today("13:58:01") };

const NOW: Readonly<Record<FixtureName, number>> = {
	idle: today("14:20:00"),
	invalid: today("14:20:30"),
	running: RUN14.started + 31 * SECOND,
	streaming: RUN14.started + 44 * SECOND,
	done: today("14:03:30"),
	failed: today("13:58:40"),
	stopped: today("14:03:00"),
	none: today("09:00:00"),
	"none-done": today("09:10:05"),
	small: today("10:10:00"),
	"small-done": today("10:12:30"),
	large: today("15:00:00"),
	compare: today("14:03:30"),
	runs: today("14:03:30"),
	uploading: today("14:20:40"),
	reopen: today("14:30:50"),
	"next-files": today("14:30:58"),
	recent: today("14:31:01"),
	series: today("14:31:40"),
	"series-failed": today("14:32:32"),
	queued: today("14:31:44"),
	"after-run": today("14:31:44"),
	"small-offer": today("10:15:21"),
	"small-reset": today("10:15:21"),
	"none-cap": today("09:12:05") + 200,
	"hosted-files": today("14:20:00"),
	shortcuts: today("14:31:42"),
	presets: today("14:03:30"),
	"preset-save": today("14:31:42"),
	"pick-again": today("14:05:00"),
	asking: RUN14.started + 39 * SECOND,
	leave: today("14:31:41"),
};

/** The moment a fixture shows: run clocks, message expiry and "Today 14:02" read against it. */
export function fixtureClock(name: FixtureName): SessionClock {
	return { now: NOW[name], today: FIXTURE_TODAY };
}

// ─── Viewer and hosts ───────────────────────────────────────────────────────

/** `flpDateLocale('en-GB')` as Main reads dates (FLP_DATE_MAIN), written out so no Intl data is needed. */
export const FIXTURE_DATE_LOCALE: DateLocale = {
	order: "dmy",
	sep: "/",
	months: [
		["january", "jan"],
		["february", "feb"],
		["march", "mar"],
		["april", "apr"],
		["may"],
		["june", "jun"],
		["july", "jul"],
		["august", "aug"],
		["september", "sept"],
		["october", "oct"],
		["november", "nov"],
		["december", "dec"],
	],
	words: { today: ["today"], yesterday: ["yesterday"], tomorrow: ["tomorrow"] },
};

/** Main's viewer: macOS, en-GB date habits, decimal sign ".". */
export const FIXTURE_VIEWER: ViewerHabits = {
	mac: true,
	locale: "en-GB",
	dateLocale: FIXTURE_DATE_LOCALE,
	decimalSign: ".",
};

/** The desktop app: device memory under profile:local, temporary uploads with FlowPaths, next files. */
export const FIXTURE_APP_HOST: HostCapabilities = {
	kind: "app",
	presentation: "page",
	persistence: "device",
	memoryScope: "profile:local",
	uploads: "temporary",
	flowPathFiles: true,
	nextFiles: true,
	stop: "cancel",
	inlineFileLimitBytes: null,
	inlineRoomBytes: null,
	warnFileBytes: FORM_LIMITS.warnFileBytes,
	fixedTarget: null,
	hasToolbar: true,
};

/** (2 MB − 64 KB) × 3/4: what one hosted run can carry inline (`flpRoom`). */
const HOSTED_ROOM_BYTES = Math.floor(
	((FORM_LIMITS.hostedRequestBytes - FORM_LIMITS.hostedReserveBytes) * 3) / 4,
);

/** A public hosted link: session memory, inline files, no FlowPath, one file per run, Stop only detaches. */
export const FIXTURE_HOSTED_HOST: HostCapabilities = {
	kind: "hosted",
	presentation: "page",
	persistence: "session",
	memoryScope: null,
	uploads: "inline",
	flowPathFiles: false,
	nextFiles: false,
	stop: "detach",
	inlineFileLimitBytes: HOSTED_ROOM_BYTES,
	inlineRoomBytes: HOSTED_ROOM_BYTES,
	warnFileBytes: null,
	fixedTarget: "remote",
	hasToolbar: true,
};

/** App host: resolved to this device, 3 at a time. Hosted: the cloud, 2 until the tier is known. */
const QUEUE_BY_HOST: Readonly<Record<FormHostKind, QueueState>> = {
	app: {
		target: "local",
		tierLimit: null,
		cap: FORM_LIMITS.localParallel,
		hold: null,
		retryAt: null,
	},
	hosted: {
		target: "remote",
		tierLimit: null,
		cap: FORM_LIMITS.unknownTierParallel,
		hold: null,
		retryAt: null,
	},
	service: {
		target: "local",
		tierLimit: null,
		cap: FORM_LIMITS.localParallel,
		hold: null,
		retryAt: null,
	},
};

// ─── Fields ─────────────────────────────────────────────────────────────────

interface FieldSpec {
	readonly name: string;
	readonly label: string;
	readonly kind: FieldKind;
	readonly dataType: string;
	readonly valueType?: string;
	readonly optional?: boolean;
	readonly help?: string;
	/** The stored default, in rail shape. */
	readonly def?: FieldValue;
	/** An optional field without a stored default, seeded as today's form does (a date is today). */
	readonly seeded?: FieldValue;
	readonly options?: readonly string[];
	readonly integer?: boolean;
	readonly fileMode?: FileMode;
	readonly itemKind?: "text" | "number" | "date";
	readonly props?: readonly FieldSpec[];
}

const NEVER_REQUIRED: ReadonlySet<FieldKind> = new Set([
	"bool",
	"group",
	"pairs",
]);
const SHORT_KINDS: ReadonlySet<FieldKind> = new Set(["number", "date", "bool"]);
const EMPTY_BY_KIND: Readonly<Record<FieldKind, FieldValue>> = {
	text: "",
	number: "",
	bool: false,
	date: "",
	choice: "",
	chips: [],
	file: null,
	files: [],
	group: {},
	pairs: [],
	json: "",
	unsupported: null,
};

/** As model/fields.ts decides it: numbers, dates, switches and choices of at most two options. */
function isShort(spec: FieldSpec) {
	if (SHORT_KINDS.has(spec.kind)) return true;
	return spec.kind === "choice" && (spec.options ?? []).length <= 2;
}

function startingValue(spec: FieldSpec): FieldValue {
	if (spec.def !== undefined) return spec.def;
	if (spec.seeded !== undefined) return spec.seeded;
	return EMPTY_BY_KIND[spec.kind];
}

function fieldOf(
	spec: FieldSpec,
	index: number,
	group: string | null,
): WorkbenchField {
	const name = spec.name;
	return {
		key: group === null ? name : `${group}${FIELD_KEY_SEPARATOR}${name}`,
		name,
		label: spec.label,
		help: spec.help ?? null,
		kind: spec.kind,
		dataType: spec.dataType,
		valueType: spec.valueType ?? "Normal",
		required: !spec.optional && !NEVER_REQUIRED.has(spec.kind),
		sensitive: false,
		defaultOmitted: false,
		defaultValue: startingValue(spec),
		hasDefault: spec.def !== undefined || spec.seeded !== undefined,
		options: spec.options ?? null,
		range: null,
		step: null,
		integer: spec.integer === true,
		fileMode: spec.fileMode ?? null,
		itemKind: spec.itemKind ?? null,
		dateFormat: null,
		props: (spec.props ?? []).map((prop, at) => fieldOf(prop, at, name)),
		short: isShort(spec),
		index,
	};
}

const SMALL_SPECS: readonly FieldSpec[] = [
	{ name: "order", label: "Order number", kind: "text", dataType: "String" },
	{
		name: "quantity",
		label: "Quantity",
		kind: "number",
		dataType: "Integer",
		integer: true,
		def: "1",
	},
	{
		name: "receipt",
		label: "Receipt",
		kind: "file",
		dataType: "Struct",
		fileMode: "flowpath",
	},
];

const MEDIUM_SPECS: readonly FieldSpec[] = [
	{
		name: "invoice_file",
		label: "Invoice",
		kind: "file",
		dataType: "Struct",
		fileMode: "flowpath",
		help: "PDF or scan of the invoice.",
	},
	{
		name: "supporting_documents",
		label: "Supporting documents",
		kind: "files",
		dataType: "Struct",
		valueType: "Array",
		fileMode: "flowpath",
		optional: true,
		help: "Delivery notes or order confirmations used to cross-check the invoice.",
	},
	{
		name: "vendor_name",
		label: "Vendor",
		kind: "text",
		dataType: "String",
		help: "Name as it appears in your supplier master data.",
	},
	{
		name: "invoice_date",
		label: "Invoice date",
		kind: "date",
		dataType: "Date",
	},
	{
		name: "expected_total",
		label: "Expected total",
		kind: "number",
		dataType: "Float",
		optional: true,
		def: "0",
		help: "Gross amount from the purchase order. Leave at 0 to skip the comparison.",
	},
	{
		name: "max_pages",
		label: "Max pages",
		kind: "number",
		dataType: "Integer",
		integer: true,
		optional: true,
		def: "20",
		help: "Pages beyond this number are ignored.",
	},
	{
		name: "run_ocr",
		label: "Run OCR",
		kind: "bool",
		dataType: "Boolean",
		optional: true,
		def: true,
		help: "Turn on for scanned or photographed invoices.",
	},
	{
		name: "cost_centers",
		label: "Cost centers",
		kind: "chips",
		dataType: "String",
		valueType: "Array",
		itemKind: "text",
		optional: true,
		def: ["4400", "4410"],
		help: "Cost centers the line items may be booked to.",
	},
	{
		name: "payment_terms",
		label: "Payment terms",
		kind: "group",
		dataType: "Struct",
		optional: true,
		help: "Terms agreed with the vendor. The invoice is flagged when it states something else.",
		def: {
			currency: "EUR",
			net_days: "14",
			discount_percent: "2",
			discount_days: "14",
		},
		props: [
			{
				name: "currency",
				label: "Currency",
				kind: "choice",
				dataType: "String",
				options: ["EUR", "USD", "CHF", "GBP"],
				def: "EUR",
			},
			{
				name: "net_days",
				label: "Net days",
				kind: "number",
				dataType: "Integer",
				integer: true,
				def: "14",
			},
			{
				name: "discount_percent",
				label: "Discount percent",
				kind: "number",
				dataType: "Float",
				optional: true,
				def: "2",
			},
			{
				name: "discount_days",
				label: "Discount days",
				kind: "number",
				dataType: "Integer",
				integer: true,
				optional: true,
				def: "14",
			},
		],
	},
];

const LARGE_SPECS: readonly FieldSpec[] = [
	{
		name: "portal_url",
		label: "Portal URL",
		kind: "text",
		dataType: "String",
		help: "Start page of the supplier portal.",
	},
	{
		name: "customer_number",
		label: "Customer number",
		kind: "text",
		dataType: "String",
	},
	{
		name: "order_numbers",
		label: "Order numbers",
		kind: "chips",
		dataType: "String",
		valueType: "Array",
		itemKind: "text",
		help: "Orders to open and verify.",
	},
	{
		name: "date_from",
		label: "From",
		kind: "date",
		dataType: "Date",
		help: "Only orders changed on or after this day.",
	},
	{
		name: "date_to",
		label: "To",
		kind: "date",
		dataType: "Date",
		optional: true,
		seeded: FIXTURE_TODAY,
	},
	{
		name: "browser_type",
		label: "Browser type",
		kind: "choice",
		dataType: "String",
		optional: true,
		def: "Chrome",
		options: ["Chrome", "Edge"],
		help: "Chrome or Edge. Firefox and Safari are not supported yet.",
	},
	{
		name: "headless",
		label: "Headless",
		kind: "bool",
		dataType: "Boolean",
		optional: true,
		def: true,
		help: "Run the browser without a visible window.",
	},
	{
		name: "viewport_width",
		label: "Viewport width",
		kind: "number",
		dataType: "Integer",
		integer: true,
		optional: true,
		def: "1920",
		help: "Page width in CSS pixels.",
	},
	{
		name: "viewport_height",
		label: "Viewport height",
		kind: "number",
		dataType: "Integer",
		integer: true,
		optional: true,
		def: "1080",
		help: "Page height in CSS pixels.",
	},
	{
		name: "page_load_timeout",
		label: "Page load timeout (s)",
		kind: "number",
		dataType: "Integer",
		integer: true,
		optional: true,
		def: "30",
		help: "At least 1 second.",
	},
	{
		name: "locale",
		label: "Locale",
		kind: "text",
		dataType: "String",
		optional: true,
		def: "",
		help: "Browser language such as de-DE. Empty keeps the default.",
	},
	{
		name: "proxy_server",
		label: "Proxy server",
		kind: "text",
		dataType: "String",
		optional: true,
		def: "",
		help: "Such as http://host:8080. Proxy credentials are not supported.",
	},
	{
		name: "ignore_https_errors",
		label: "Ignore HTTPS errors",
		kind: "bool",
		dataType: "Boolean",
		optional: true,
		def: false,
		help: "Accept invalid or self-signed certificates.",
	},
	{
		name: "max_retries",
		label: "Max retries",
		kind: "number",
		dataType: "Integer",
		integer: true,
		optional: true,
		def: "2",
		help: "How often a failed page is opened again before the order is marked as failed.",
	},
	{
		name: "slow_page_threshold",
		label: "Slow page threshold (s)",
		kind: "number",
		dataType: "Float",
		optional: true,
		def: "2.5",
		help: "Pages slower than this are listed in the report.",
	},
	{
		name: "min_quality_score",
		label: "Minimum quality score",
		kind: "number",
		dataType: "Float",
		optional: true,
		def: "0.85",
		help: "Between 0 and 1. Below this score the run is reported as failed.",
	},
	{
		name: "compare_with_baseline",
		label: "Compare with baseline",
		kind: "bool",
		dataType: "Boolean",
		optional: true,
		def: true,
	},
	{
		name: "baseline_export",
		label: "Baseline export",
		kind: "file",
		dataType: "Struct",
		fileMode: "flowpath",
		optional: true,
		help: "Last accepted export (CSV or XLSX) to compare against.",
	},
	{
		name: "checklist",
		label: "Checklist",
		kind: "group",
		dataType: "Struct",
		optional: true,
		help: "Which parts of every order are compared.",
		def: {
			prices: true,
			delivery_dates: true,
			documents: false,
			tolerance_percent: "1.5",
		},
		props: [
			{
				name: "prices",
				label: "Prices",
				kind: "bool",
				dataType: "Boolean",
				def: true,
			},
			{
				name: "delivery_dates",
				label: "Delivery dates",
				kind: "bool",
				dataType: "Boolean",
				def: true,
			},
			{
				name: "documents",
				label: "Documents",
				kind: "bool",
				dataType: "Boolean",
				optional: true,
				def: false,
			},
			{
				name: "tolerance_percent",
				label: "Tolerance percent",
				kind: "number",
				dataType: "Float",
				optional: true,
				def: "1.5",
			},
		],
	},
	{
		name: "capture_screenshots",
		label: "Capture screenshots",
		kind: "bool",
		dataType: "Boolean",
		optional: true,
		def: true,
		help: "Saves one screenshot per checked order.",
	},
	{
		name: "record_video",
		label: "Record video",
		kind: "bool",
		dataType: "Boolean",
		optional: true,
		def: false,
	},
	{
		name: "reference_documents",
		label: "Reference documents",
		kind: "files",
		dataType: "Struct",
		valueType: "Array",
		fileMode: "flowpath",
		optional: true,
		help: "Order confirmations or price lists the portal is checked against.",
	},
	{
		name: "skip_orders",
		label: "Orders to skip",
		kind: "chips",
		dataType: "String",
		valueType: "Array",
		itemKind: "text",
		optional: true,
		def: [],
	},
	{
		name: "report_title",
		label: "Report title",
		kind: "text",
		dataType: "String",
	},
	{
		name: "report_language",
		label: "Report language",
		kind: "text",
		dataType: "String",
		optional: true,
		def: "en",
		help: "Language code such as en or de.",
	},
	{
		name: "send_email",
		label: "Send report by e-mail",
		kind: "bool",
		dataType: "Boolean",
		optional: true,
		def: false,
	},
	{
		name: "recipients",
		label: "Recipients",
		kind: "chips",
		dataType: "String",
		valueType: "Array",
		itemKind: "text",
		optional: true,
		def: [],
		help: "People who receive the report.",
	},
	{
		name: "email_subject",
		label: "E-mail subject",
		kind: "text",
		dataType: "String",
		optional: true,
		def: "",
		help: "Leave empty to use the report title.",
	},
	{
		name: "notes",
		label: "Notes for the reviewer",
		kind: "text",
		dataType: "String",
		optional: true,
		def: "",
		help: "Added to the first page of the report.",
	},
];

// ─── Forms ──────────────────────────────────────────────────────────────────

interface FormSpec {
	readonly appId: string;
	readonly eventId: string;
	readonly nodeId: string;
	/** The app's name in the host header. */
	readonly appName: string;
	readonly name: string;
	readonly description: string;
	/** Configured routes with their labels (the routed event's name, "Home" for "/"). */
	readonly routes: readonly (readonly [string, string])[];
	readonly specs: readonly FieldSpec[];
}

const FORM_SPECS: Readonly<Record<FormKey, FormSpec>> = {
	none: {
		appId: "app-support-copilot",
		eventId: "event-triage-request",
		nodeId: "node-triage-request",
		appName: "Customer Support Copilot",
		name: "Triage selected request",
		description:
			"Analyze the selected customer request and prepare a response.",
		routes: [["/chat", "Support chat"]],
		specs: [],
	},
	small: {
		appId: "app-shop-assistant",
		eventId: "event-return-request",
		nodeId: "node-return-request",
		appName: "Shop Assistant",
		name: "Return request",
		description: "Asks the shop to take an order back.",
		routes: [],
		specs: SMALL_SPECS,
	},
	medium: {
		appId: "app-invoice-ai",
		eventId: "event-extract-invoice",
		nodeId: "node-extract-invoice",
		appName: "Invoice AI",
		name: "Extract invoice",
		description:
			"Upload an invoice and its supporting documents. The flow reads every page, extracts the line items and checks the totals against your purchase order.",
		routes: [
			["/", "Home"],
			["/review", "Review queue"],
		],
		specs: MEDIUM_SPECS,
	},
	large: {
		appId: "app-supplier-portal-checks",
		eventId: "event-supplier-portal-quality-run",
		nodeId: "node-supplier-portal-quality-run",
		appName: "Supplier Portal Checks",
		name: "Supplier portal quality run",
		description:
			"Signs in to the supplier portal, opens the selected orders, compares what it finds with the last accepted export and writes a quality report. A run takes 5 to 15 minutes depending on the number of orders.",
		routes: [
			["/", "Home"],
			["/reports", "Reports"],
			["/orders", "Orders"],
		],
		specs: LARGE_SPECS,
	},
};

const fieldsOfSpec = (spec: FormSpec) =>
	spec.specs.map((field, index) => fieldOf(field, index, null));

/** The four forms' fields as `fieldsFromEvent` should read them (FlowPath pins are file fields). */
export const FIXTURE_FIELDS: Readonly<
	Record<FormKey, readonly WorkbenchField[]>
> = {
	none: fieldsOfSpec(FORM_SPECS.none),
	small: fieldsOfSpec(FORM_SPECS.small),
	medium: fieldsOfSpec(FORM_SPECS.medium),
	large: fieldsOfSpec(FORM_SPECS.large),
};

/** A form model on a host (the app host unless given). */
export function fixtureForm(
	key: FormKey,
	host: HostCapabilities = FIXTURE_APP_HOST,
): FormModel {
	const spec = FORM_SPECS[key];
	return {
		appId: spec.appId,
		eventId: spec.eventId,
		nodeId: spec.nodeId,
		name: spec.name,
		description: spec.description,
		fields: FIXTURE_FIELDS[key],
		contentKey: `fixture:${key}`,
		submitLabel: null,
		routes: spec.routes.map(([route]) => route),
		eventRoute: null,
		host,
		viewer: FIXTURE_VIEWER,
	};
}

/** Route → label for `RouteNav.labels` (and the harness's host header). */
export function fixtureRouteLabels(
	key: FormKey,
): Readonly<Record<string, string>> {
	return Object.fromEntries(FORM_SPECS[key].routes);
}

/** Every field's starting value without a preset, by field name. */
export function fixtureDefaults(key: FormKey): FieldValues {
	return Object.fromEntries(
		FIXTURE_FIELDS[key].map((field) => [field.name, field.defaultValue]),
	);
}

// ─── Files ──────────────────────────────────────────────────────────────────

interface FileSpec {
	readonly id: string;
	readonly name: string;
	readonly size: number | null;
	readonly type: string;
}

interface InvoiceSpec extends FileSpec {
	readonly date: string;
}

const PDF = "application/pdf";
const JPEG = "image/jpeg";

const INVOICE_0917: FileSpec = {
	id: "slot-invoice-0917",
	name: "invoice-RE-2026-0917.pdf",
	size: 1284096,
	type: PDF,
};
/** The same invoice picked again with the folder (⌘O, ⌘A): a new File, left out because run 14 had it. */
const PICKED_0917: FileSpec = { ...INVOICE_0917, id: "slot-pick-0917" };
const INVOICE_0911: FileSpec = {
	id: "slot-invoice-0911",
	name: "invoice-RE-2026-0911.pdf",
	size: null,
	type: PDF,
};
const LIEFERSCHEIN: FileSpec = {
	id: "slot-ls-77120",
	name: "Lieferschein LS-77120.pdf",
	size: 318464,
	type: PDF,
};
const PO_48213: FileSpec = {
	id: "slot-po-48213",
	name: "PO-48213.pdf",
	size: 96256,
	type: PDF,
};

/** FLP_INVOICES: the ten Nordwind invoices of the benchmark; 0921 is the one whose run fails. */
const INVOICES: readonly InvoiceSpec[] = [
	["0918", 1198080, "2026-09-18"],
	["0919", 1361920, "2026-09-21"],
	["0920", 1003520, "2026-09-21"],
	["0921", 1247232, "2026-09-22"],
	["0922", 1144832, "2026-09-23"],
	["0923", 4089446, "2026-09-24"],
	["0924", 1310720, "2026-09-25"],
	["0925", 1093632, "2026-09-28"],
	["0926", 1222656, "2026-09-29"],
	["0927", 1175552, "2026-09-30"],
].map(([number, size, date]) => ({
	id: `slot-invoice-${number}`,
	name: `invoice-RE-2026-${number}.pdf`,
	size: size as number,
	type: PDF,
	date: date as string,
}));

/** FLP_PARCELS: the three parcels of Return request, one run each. */
const PARCELS = [
	{ order: "48213-7", quantity: "2", size: 412518 },
	{ order: "48231-1", quantity: "1", size: 389120 },
	{ order: "48240-3", quantity: "1", size: 455680 },
].map((parcel) => ({
	...parcel,
	receipt: {
		id: `slot-receipt-${parcel.order}`,
		name: `receipt-${parcel.order}.jpg`,
		size: parcel.size,
		type: JPEG,
	} satisfies FileSpec,
}));

function flowPathOf(key: FormKey, file: FileSpec): FlowPath {
	const form = FORM_SPECS[key];
	const request = file.id.replace(/^slot-/, "request-");
	return {
		path: `tmp/global/apps/${form.appId}/events/${form.eventId}/requests/${request}/0001-${file.name.replaceAll(" ", "_")}`,
		store_ref: REQUEST_FILES_STORE_REF,
		cache_store_ref: null,
	};
}

const SLOT_BASE = {
	progress: null,
	ref: null,
	error: null,
	sentAt: null,
	expiresAt: null,
} as const;

/** Uploaded: the desktop staged it in the temporary store and returned a FlowPath. */
function sentSlot(key: FormKey, file: FileSpec, sentAt: number): FileSlot {
	return {
		...SLOT_BASE,
		...file,
		state: "sent",
		ref: { kind: "flowpath", flowPath: flowPathOf(key, file), url: null },
		sentAt,
	};
}

function sendingSlot(file: FileSpec, progress: number): FileSlot {
	return { ...SLOT_BASE, ...file, state: "sending", progress };
}

function waitingSlot(file: FileSpec): FileSlot {
	return { ...SLOT_BASE, ...file, state: "waiting" };
}

/** A file of a run read back from this device's history: a name to pick again, not a value (S4). */
function reminderSlot(id: string, name: string, size: number | null): FileSlot {
	return { ...SLOT_BASE, id, name, size, type: null, state: "reminder" };
}

// ─── Run output ─────────────────────────────────────────────────────────────

/** FL_STEPS: the plan a run of Extract invoice streams. */
const STEPS = [
	{
		title: "Read documents",
		detail: "Loading the invoice and 2 supporting documents",
		note: "invoice-RE-2026-0917.pdf has 14 pages. 2 supporting documents loaded.",
	},
	{
		title: "Run OCR",
		detail: "Recognising text on 6 scanned pages",
		note: "6 of 14 pages are scans. Text recognised with 0.94 confidence.",
	},
	{
		title: "Extract fields",
		detail: "Vendor, dates, totals and line items",
		note: "Vendor, invoice date, due date, VAT ID and 4 line items found.",
	},
	{
		title: "Match purchase order",
		detail: "Comparing 4 line items with PO-48213",
		note: "3 of 4 line items matched. Line 4 (Express delivery, 2 x 300.00) has no position on the purchase order. Checking delivery note LS-77120 for a second express shipment.",
	},
	{
		title: "Check totals",
		detail: "Recalculating net, VAT and gross",
		note: "Net, VAT and gross add up. Gross is 714.00 above the expected total.",
	},
	{
		title: "Write report",
		detail: "Creating the summary and the export files",
		note: "Summary written. 12 files created.",
	},
] as const;

function stepRef(number: number): StepRef {
	return { number, title: STEPS[number - 1].title };
}

/**
 * The plan up to step `reached`; that step is `last`. Rich runs (14 and 13 in the canvas) carry
 * descriptions and the active step's message; the series runs show titles only.
 */
function stepsOf(
	reached: number,
	last: RunStepState,
	rich: boolean,
): RunStep[] {
	return STEPS.slice(0, reached).map((step, index) => {
		const state: RunStepState = index === reached - 1 ? last : "done";
		const message = rich && state === "active" ? step.note : null;
		return {
			id: `step-${index + 1}`,
			number: index + 1,
			title: step.title,
			detail: rich ? step.detail : null,
			message,
			state,
		};
	});
}

const ANSWER_TABLE = `| # | Description | Qty | Unit price | Net |
|---|---|--:|--:|--:|
| 1 | Pallet transport Hamburg–Munich | 12 | € 410.00 | € 4,920.00 |
| 2 | Cold-chain surcharge | 12 | € 85.00 | € 1,020.00 |
| 3 | Customs handling (flat fee) | 1 | € 3,950.00 | € 3,950.00 |
| 4 | Express delivery | 2 | € 300.00 | € 600.00 |`;

const ANSWER_HEAD = `## Invoice RE-2026-0917 extracted

**Vendor:** Nordwind Logistik GmbH · **Invoice date:** 17 Sep 2026 · **Due:** 17 Oct 2026

All 14 pages were read and four line items were found. Three of them match purchase order PO-48213. The gross total is **€ 12,483.10**, which is € 714.00 above the expected total you entered (€ 11,769.10).

${ANSWER_TABLE}

`;

/** FL_ANSWER as the markdown a run streams. */
export const FIXTURE_ANSWER = `${ANSWER_HEAD}Net € 10,490.00 · VAT 19 % € 1,993.10 · Gross € 12,483.10

### Needs your attention

- **Line 4 is not on the purchase order.** The express delivery fee (€ 600.00 net, € 714.00 gross) has no matching position, and delivery note LS-77120 mentions one express shipment, not two.
- **Payment terms differ.** The invoice says 30 days net; the form says 14 days with 2 % discount.
- **Cost center missing for line 3.** Lines 1 and 2 went to \`4400\`, line 4 to \`4410\`.

### What was checked

1. Vendor name and VAT ID against the supplier master data
2. Arithmetic of every line and of the VAT amount
3. Quantities against the delivery note
4. Duplicates among invoices of the last 90 days: none found

The extracted data is attached as \`invoice-RE-2026-0917.json\` and \`line-items.csv\`. Approve the invoice in the review queue, or send the generated query letter to the vendor.`;

/** What has streamed in `streaming` (`flReveal(FL_ANSWER, 520)`): up to "Net € 10,490". */
const ANSWER_STREAMED = `${ANSWER_HEAD}Net € 10,490`;

/** FL_ANSWER_EARLIER: run 12's answer. */
const ANSWER_EARLIER = `## Invoice RE-2026-0911 extracted

**Vendor:** Nordwind Logistik GmbH · **Invoice date:** 11 Sep 2026 · **Due:** 25 Sep 2026

All 3 pages were read and two line items were found. Both match purchase order PO-48190. The gross total is **€ 6,188.00**, equal to the expected total.

| # | Description | Qty | Unit price | Net |
|---|---|--:|--:|--:|
| 1 | Pallet transport Hamburg–Munich | 10 | € 410.00 | € 4,100.00 |
| 2 | Cold-chain surcharge | 10 | € 110.00 | € 1,100.00 |

Nothing needs your attention. The invoice can be approved.`;

const DATE_TEXT: Readonly<Record<string, string>> = {
	"2026-09-18": "18 Sep 2026",
	"2026-09-21": "21 Sep 2026",
	"2026-09-22": "22 Sep 2026",
	"2026-09-23": "23 Sep 2026",
	"2026-09-24": "24 Sep 2026",
	"2026-09-25": "25 Sep 2026",
	"2026-09-28": "28 Sep 2026",
	"2026-09-29": "29 Sep 2026",
	"2026-09-30": "30 Sep 2026",
};

/** `flpAnswerFor(inv)`: a finished series run's answer. */
function seriesAnswer(invoice: InvoiceSpec) {
	const number = invoice.name.replace(/^invoice-/, "").replace(/\.pdf$/, "");
	return `## Invoice ${number} extracted

**Vendor:** Nordwind Logistik GmbH · **Invoice date:** ${DATE_TEXT[invoice.date]}

Nothing needs your attention. The invoice can be approved.`;
}

/** FL_RESULT: what run 14 returned through "Return Generic Result". */
export const FIXTURE_RESULT = {
	invoice_number: "RE-2026-0917",
	status: "needs_review",
	vendor: {
		name: "Nordwind Logistik GmbH",
		vat_id: "DE123456789",
		supplier_id: "S-10482",
		verified: true,
	},
	invoice_date: "2026-09-17",
	due_date: "2026-10-17",
	currency: "EUR",
	totals: {
		net: 10490.0,
		vat_rate: 0.19,
		vat: 1993.1,
		gross: 12483.1,
		expected_gross: 11769.1,
		difference: 714.0,
	},
	line_items: [
		{
			position: 1,
			description: "Pallet transport Hamburg–Munich",
			quantity: 12,
			unit_price: 410.0,
			net: 4920.0,
			cost_center: "4400",
			po_match: true,
		},
		{
			position: 2,
			description: "Cold-chain surcharge",
			quantity: 12,
			unit_price: 85.0,
			net: 1020.0,
			cost_center: "4400",
			po_match: true,
		},
		{
			position: 3,
			description: "Customs handling (flat fee)",
			quantity: 1,
			unit_price: 3950.0,
			net: 3950.0,
			cost_center: null,
			po_match: true,
		},
		{
			position: 4,
			description: "Express delivery",
			quantity: 2,
			unit_price: 300.0,
			net: 600.0,
			cost_center: "4410",
			po_match: false,
		},
	],
	findings: [
		{
			code: "po_mismatch",
			severity: "high",
			line: 4,
			message: "No matching position on PO-48213.",
		},
		{
			code: "terms_mismatch",
			severity: "medium",
			line: null,
			message:
				"Invoice says 30 days net; agreed terms are 14 days with 2 % discount.",
		},
		{
			code: "cost_center_missing",
			severity: "low",
			line: 3,
			message: "No cost center assigned.",
		},
	],
	checks: {
		arithmetic_ok: true,
		duplicate: false,
		pages_read: 14,
		ocr_used: true,
		confidence: 0.94,
	},
	duration_ms: 48211,
} as const;

/** FL_RESULT_SMALL.raw: what a Return request run returns. */
export const FIXTURE_RESULT_SMALL = {
	return_number: "RET-20931",
	status: "accepted",
	label: "sent by e-mail",
} as const;

/** Attachments point here; the visual harness answers these URLs with placeholders. */
export const FIXTURE_FILES_ORIGIN = "https://files.flow-like.test";

const MIME_BY_EXTENSION: Readonly<Record<string, string>> = {
	pdf: PDF,
	json: "application/json",
	csv: "text/csv",
	xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
	docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
	png: "image/png",
	jpg: JPEG,
	mp4: "video/mp4",
	mp3: "audio/mpeg",
	zip: "application/zip",
};

/** FL_FILES: the twelve files run 14 attached (two without a size, one with a long name). */
const RETURNED_FILES: readonly (readonly [string, number | null])[] = [
	["invoice-RE-2026-0917.pdf", 1284096],
	["invoice-RE-2026-0917.json", 6412],
	["line-items.csv", 1873],
	["reconciliation.xlsx", 48640],
	["query-letter-nordwind.docx", 31744],
	[
		"2026-09-17_Nordwind-Logistik-GmbH_Rechnung-RE-2026-0917_geprueft_mit-Anmerkungen.pdf",
		null,
	],
	["page-01-header.png", 412518],
	["page-03-line-items.png", 655360],
	["page-14-totals.jpg", 238592],
	["portal-walkthrough.mp4", 18874368],
	["summary-readout.mp3", null],
	["evidence-bundle.zip", 24117248],
];

function returnedFiles(backendRunId: string): IAttachment[] {
	return RETURNED_FILES.map(([name, size]) => ({
		url: `${FIXTURE_FILES_ORIGIN}/runs/${backendRunId}/${encodeURIComponent(name)}`,
		name,
		size,
		type: MIME_BY_EXTENSION[name.split(".").pop() ?? ""],
	}));
}

const LIVE_TERMINAL: TerminalSignals = {
	runInitiated: true,
	errorMessage: null,
	completedStatus: null,
	rejectedStage: null,
	logLevel: null,
	durationMs: null,
};

/** FL_FAILURE: the raw line a builder sees; the run ends at step 2 after 12 s. */
const FAILURE_DETAIL = "Failed to execute node: Array value is not an array";
const FAILURE_RUN_ID = "run_7f3a91c2";

function endedTerminal(
	completedStatus: "completed" | "failed" | "cancelled",
	seconds: number,
): TerminalSignals {
	const failed = completedStatus === "failed";
	return {
		...LIVE_TERMINAL,
		completedStatus,
		errorMessage: failed ? FAILURE_DETAIL : null,
		logLevel: failed ? 3 : 1,
		durationMs: seconds * SECOND,
	};
}

const SUCCEEDED: RunOutcome = { kind: "succeeded" };
const STOPPED: RunOutcome = { kind: "stopped" };

function failedOutcome(backendRunId: string): RunOutcome {
	return {
		kind: "failed",
		failure: "flow",
		message: FAILURE_DETAIL,
		detail: `${FAILURE_DETAIL}\nRun ${backendRunId} · log level 3`,
	};
}

const OUTPUT_BASE: RunOutput = {
	eventCount: 1,
	steps: [],
	answer: "",
	reasoning: null,
	attachments: [],
	result: null,
	interactions: [],
	terminal: LIVE_TERMINAL,
};

/** A run's output; the event count is the run start, two events per step and the end. */
function outputOf(parts: Partial<RunOutput>): RunOutput {
	const output = { ...OUTPUT_BASE, ...parts };
	const ended = output.terminal.completedStatus === null ? 0 : 1;
	return {
		...output,
		eventCount: 1 + output.steps.length * 2 + output.attachments.length + ended,
	};
}

const SUMMARY_BASE: RunSummary = {
	firstLine: null,
	stepCount: 0,
	stepReached: null,
	fileCount: 0,
	hasAnswer: false,
	hasResult: false,
};

/** What lists show of a run that reached step `reached` (0: none yet) without an answer. */
function stepSummary(reached: number): RunSummary {
	if (reached === 0) return SUMMARY_BASE;
	return { ...SUMMARY_BASE, stepCount: reached, stepReached: stepRef(reached) };
}

// ─── Runs ───────────────────────────────────────────────────────────────────

const runId = (n: number) => `run-${n}`;

/** A stable backend run id per run number (run 13 keeps FL_FAILURE's). */
function backendIdOf(n: number) {
	if (n === 13) return FAILURE_RUN_ID;
	const hash = (Math.imul(n + 0x5c2d, 0x9e3779b1) >>> 0).toString(16);
	return `run_${hash.padStart(8, "0")}`;
}

interface SessionRunSpec {
	readonly n: number;
	readonly status: RunStatus;
	readonly createdAt: number;
	readonly startedAt: number | null;
	readonly endedAt: number | null;
	readonly values: Readonly<Record<string, CopyValue>>;
	readonly perRun: readonly string[];
	readonly replaced: readonly string[];
	readonly presetName: string | null;
	readonly output: RunOutput | null;
	readonly outcome: RunOutcome | null;
	readonly summary: RunSummary;
	readonly stopRequested: boolean;
	readonly pendingSlotIds: readonly string[];
	readonly failedAt: StepRef | null;
	readonly unseenFailure: boolean;
}

const SESSION_RUN_DEFAULTS = {
	startedAt: null,
	endedAt: null,
	perRun: [],
	replaced: [],
	presetName: null,
	output: null,
	outcome: null,
	summary: SUMMARY_BASE,
	stopRequested: false,
	pendingSlotIds: [],
	failedAt: null,
	unseenFailure: false,
} as const;

type SessionRunInput = Pick<
	SessionRunSpec,
	"n" | "status" | "createdAt" | "values"
> &
	Partial<SessionRunSpec>;

/** A run started in this window: it runs on this device and keeps its copy (files by slot) in memory. */
function sessionRun(input: SessionRunInput): RunEntry {
	const spec: SessionRunSpec = { ...SESSION_RUN_DEFAULTS, ...input };
	const dispatched = spec.startedAt !== null;
	const id = runId(spec.n);
	return {
		id,
		n: spec.n,
		origin: "session",
		status: spec.status,
		createdAt: spec.createdAt,
		startedAt: spec.startedAt,
		endedAt: spec.endedAt,
		copy: {
			values: spec.values,
			presetName: spec.presetName,
			leftAsIs: false,
			perRun: spec.perRun,
			replaced: spec.replaced,
		},
		target: "local",
		streamId: dispatched ? `stream-${id}` : null,
		backendRunId: dispatched ? backendIdOf(spec.n) : null,
		output: spec.output,
		outcome: spec.outcome,
		summary: spec.summary,
		stopRequested: spec.stopRequested,
		pendingSlotIds: spec.pendingSlotIds,
		waitingForPlace: false,
		failedAt: spec.failedAt,
		unseenFailure: spec.unseenFailure,
	};
}

// ─── Extract invoice: values and history ────────────────────────────────────

const NORDWIND = "Nordwind Logistik GmbH";
const BOTH_CENTERS: readonly string[] = ["4400", "4410"];

/** Extract invoice at its defaults with some inputs changed (by field name). */
function mediumValues(changes: FieldValues): FieldValues {
	return { ...fixtureDefaults("medium"), ...changes };
}

/** FL_FILLED.medium: what the rail holds in every base artboard that shows a run. */
function filledMedium(
	invoiceSentAt = today("14:01:40"),
	documentsSentAt = invoiceSentAt,
): FieldValues {
	return mediumValues({
		invoice_file: sentSlot("medium", INVOICE_0917, invoiceSentAt),
		supporting_documents: [
			sentSlot("medium", LIEFERSCHEIN, documentsSentAt),
			sentSlot("medium", PO_48213, documentsSentAt),
		],
		vendor_name: NORDWIND,
		invoice_date: "2026-09-17",
		expected_total: "11769.10",
	});
}

/** One of FL_RUNS with the inputs `flpRunInputs` walks back for it (every date 17 Sep 2026). */
interface HistoryRow {
	readonly n: number;
	readonly status: "done" | "failed" | "stopped";
	readonly day: DayKey;
	readonly time: string;
	readonly took: number;
	readonly invoice: string;
	readonly invoiceSize: number | null;
	readonly docs: boolean;
	readonly vendor: string;
	readonly total: string;
	readonly pages: string;
	readonly ocr: boolean;
	readonly centers: readonly string[];
	readonly reached: number;
	readonly firstLine: string | null;
	readonly files: number;
}

const ROW = {
	status: "done",
	invoiceSize: null,
	docs: true,
	vendor: NORDWIND,
	total: "6188.00",
	pages: "40",
	ocr: false,
	centers: ["4400"],
	reached: 6,
	firstLine: null,
	files: 0,
} as const;

const MEDIUM_HISTORY: readonly HistoryRow[] = [
	{
		...ROW,
		n: 14,
		day: "today",
		time: "14:02",
		took: 48,
		invoice: INVOICE_0917.name,
		invoiceSize: INVOICE_0917.size,
		total: "11769.10",
		pages: "20",
		ocr: true,
		centers: BOTH_CENTERS,
		firstLine:
			"Invoice RE-2026-0917 extracted. 4 line items, 3 match PO-48213.",
		files: 12,
	},
	{
		...ROW,
		n: 13,
		status: "failed",
		day: "today",
		time: "13:58",
		took: 12,
		invoice: INVOICE_0917.name,
		total: "11769.10",
		pages: "20",
		centers: BOTH_CENTERS,
		reached: 2,
	},
	{
		...ROW,
		n: 12,
		day: "today",
		time: "11:20",
		took: 41,
		invoice: INVOICE_0911.name,
		pages: "20",
		centers: BOTH_CENTERS,
		firstLine:
			"Invoice RE-2026-0911 extracted. 2 line items, both match PO-48190.",
		files: 9,
	},
	{
		...ROW,
		n: 11,
		day: "today",
		time: "09:47",
		took: 72,
		invoice: "invoice-4471-B.pdf",
		vendor: "Alpenfracht AG",
		pages: "60",
		centers: BOTH_CENTERS,
		firstLine: "Invoice 4471-B extracted. 31 line items, 29 match PO-47977.",
		files: 14,
	},
	{
		...ROW,
		n: 10,
		status: "stopped",
		day: "yesterday",
		time: "17:31",
		took: 37,
		invoice: "invoice-RE-2026-0902.pdf",
		pages: "20",
		centers: BOTH_CENTERS,
		reached: 4,
	},
	{
		...ROW,
		n: 9,
		day: "yesterday",
		time: "16:05",
		took: 52,
		invoice: "invoice-RE-2026-0902.pdf",
		centers: BOTH_CENTERS,
		firstLine:
			"Invoice RE-2026-0902 extracted. 6 line items, all match PO-48102.",
		files: 11,
	},
	{
		...ROW,
		n: 8,
		day: "yesterday",
		time: "15:40",
		took: 44,
		invoice: "invoice-RE-2026-0888.pdf",
		centers: BOTH_CENTERS,
		firstLine:
			"Invoice RE-2026-0888 extracted. 3 line items, 2 match PO-48077.",
		files: 10,
	},
	{
		...ROW,
		n: 7,
		day: "yesterday",
		time: "15:22",
		took: 46,
		invoice: "invoice-RE-2026-0888.pdf",
		firstLine:
			"Invoice RE-2026-0888 extracted. 3 line items, 1 match PO-48077.",
		files: 10,
	},
	{
		...ROW,
		n: 6,
		status: "failed",
		day: "yesterday",
		time: "10:14",
		took: 3,
		invoice: "invoice-RE-2026-0871.pdf",
		reached: 1,
	},
	{
		...ROW,
		n: 5,
		day: "yesterday",
		time: "10:02",
		took: 39,
		invoice: "invoice-RE-2026-0871.pdf",
		ocr: true,
		firstLine:
			"Invoice RE-2026-0871 extracted. 5 line items, all match PO-48031.",
		files: 10,
	},
	{
		...ROW,
		n: 4,
		day: "mon28",
		time: "16:48",
		took: 43,
		invoice: "invoice-RE-2026-0855.pdf",
		total: "2940.00",
		ocr: true,
		firstLine:
			"Invoice RE-2026-0855 extracted. 2 line items, both match PO-47990.",
		files: 9,
	},
	{
		...ROW,
		n: 3,
		day: "mon28",
		time: "16:41",
		took: 40,
		invoice: "invoice-RE-2026-0855.pdf",
		total: "0",
		ocr: true,
		firstLine:
			"Invoice RE-2026-0855 extracted. 2 line items. No expected total given.",
		files: 9,
	},
	{
		...ROW,
		n: 2,
		day: "mon28",
		time: "09:15",
		took: 58,
		invoice: "invoice-RE-2026-0840.pdf",
		total: "0",
		ocr: true,
		firstLine:
			"Invoice RE-2026-0840 extracted. 8 line items, 7 match PO-47952.",
		files: 12,
	},
	{
		...ROW,
		n: 1,
		day: "mon28",
		time: "09:03",
		took: 47,
		invoice: "invoice-RE-2026-0840.pdf",
		docs: false,
		total: "0",
		ocr: true,
		firstLine:
			"Invoice RE-2026-0840 extracted. 8 line items. No documents to compare.",
		files: 8,
	},
];

const historyRow = (n: number) =>
	MEDIUM_HISTORY.find((row) => row.n === n) as HistoryRow;

/** A history row's inputs with the given files (reminders when read back, slots in this session). */
function rowValues(
	row: HistoryRow,
	invoice: FileSlot,
	docs: readonly FileSlot[],
): FieldValues {
	return mediumValues({
		invoice_file: invoice,
		supporting_documents: row.docs ? docs : [],
		vendor_name: row.vendor,
		invoice_date: "2026-09-17",
		expected_total: row.total,
		max_pages: row.pages,
		run_ocr: row.ocr,
		cost_centers: row.centers,
	});
}

/** A history row's inputs as read back from this device: files are names to pick again. */
function storedRowValues(row: HistoryRow): FieldValues {
	const prefix = `history-${row.n}`;
	return rowValues(
		row,
		reminderSlot(`${prefix}-invoice`, row.invoice, row.invoiceSize),
		[
			reminderSlot(`${prefix}-doc-1`, LIEFERSCHEIN.name, LIEFERSCHEIN.size),
			reminderSlot(`${prefix}-doc-2`, PO_48213.name, PO_48213.size),
		],
	);
}

const ROW_OUTCOME: Readonly<Record<HistoryRow["status"], RunOutcome>> = {
	done: SUCCEEDED,
	failed: { kind: "failed", failure: "flow", message: null, detail: null },
	stopped: STOPPED,
};

function rowSummary(row: HistoryRow): RunSummary {
	if (row.status !== "done") return stepSummary(row.reached);
	return {
		firstLine: row.firstLine,
		stepCount: STEPS.length,
		stepReached: stepRef(STEPS.length),
		fileCount: row.files,
		hasAnswer: true,
		hasResult: row.n === 14,
	};
}

/** A run read back from this device: no output (answers are not stored), files to pick again. */
function historyEntry(row: HistoryRow): RunEntry {
	const createdAt = at(row.day, `${row.time}:00`);
	const startedAt = createdAt + SECOND;
	const outcome =
		row.n === 13 ? failedOutcome(FAILURE_RUN_ID) : ROW_OUTCOME[row.status];
	return {
		id: runId(row.n),
		n: row.n,
		origin: "history",
		status: row.status,
		createdAt,
		startedAt,
		endedAt: startedAt + row.took * SECOND,
		copy: {
			values: storedRowValues(row),
			presetName: null,
			leftAsIs: false,
			perRun: [],
			replaced: [],
		},
		target: null,
		streamId: null,
		backendRunId: null,
		output: null,
		outcome,
		summary: rowSummary(row),
		stopRequested: false,
		pendingSlotIds: [],
		waitingForPlace: false,
		failedAt: row.status === "failed" ? stepRef(row.reached) : null,
		unseenFailure: false,
	};
}

/** History of Extract invoice from run `newest` down to run 1, newest first. */
function mediumHistory(newest: number): RunEntry[] {
	return MEDIUM_HISTORY.filter((row) => row.n <= newest).map(historyEntry);
}

// ─── Run 14 and the other runs of Extract invoice in this session ──────────

const RUN14_SPEC = {
	n: 14,
	createdAt: RUN14.created,
	startedAt: RUN14.started,
} as const;

function run14Running(values: FieldValues): RunEntry {
	return sessionRun({
		...RUN14_SPEC,
		status: "running",
		values,
		output: outputOf({ steps: stepsOf(4, "active", true) }),
		summary: stepSummary(4),
	});
}

function run14Streaming(values: FieldValues): RunEntry {
	return sessionRun({
		...RUN14_SPEC,
		status: "streaming",
		values,
		output: outputOf({
			steps: stepsOf(6, "active", true),
			answer: ANSWER_STREAMED,
		}),
		summary: {
			...stepSummary(6),
			firstLine: "Invoice RE-2026-0917 extracted",
			hasAnswer: true,
		},
	});
}

function run14Done(values: FieldValues, presetName: string | null = null) {
	return sessionRun({
		...RUN14_SPEC,
		status: "done",
		endedAt: RUN14.started + 48 * SECOND,
		values,
		presetName,
		output: outputOf({
			steps: stepsOf(6, "done", true),
			answer: FIXTURE_ANSWER,
			result: { value: FIXTURE_RESULT },
			attachments: returnedFiles(backendIdOf(14)),
			terminal: endedTerminal("completed", 48),
		}),
		outcome: SUCCEEDED,
		summary: rowSummary(historyRow(14)),
	});
}

function run14Stopped(values: FieldValues): RunEntry {
	return sessionRun({
		...RUN14_SPEC,
		status: "stopped",
		endedAt: RUN14.started + 37 * SECOND,
		values,
		output: outputOf({
			steps: stepsOf(4, "stopped", true),
			terminal: endedTerminal("cancelled", 37),
		}),
		outcome: STOPPED,
		summary: stepSummary(4),
		stopRequested: true,
	});
}

/** An in-run question (`interaction_request`) while step 4 checks line 4; no artboard draws it. */
function lineFourQuestion(askedAt: number): IInteractionRequest {
	return {
		id: "interaction-line-4",
		name: "Line 4 is not on the purchase order",
		description:
			"Express delivery (2 x 300.00) has no position on PO-48213. How should this run book it?",
		interaction_type: {
			type: "single_choice",
			options: [
				{ id: "book-4410", label: "Book it to cost center 4410" },
				{ id: "hold", label: "Hold the invoice for review" },
			],
		},
		status: "pending",
		ttl_seconds: 900,
		expires_at: Math.floor(askedAt / SECOND) + 900,
		run_id: backendIdOf(14),
		app_id: FORM_SPECS.medium.appId,
	};
}

function run14Asking(values: FieldValues): RunEntry {
	return sessionRun({
		...RUN14_SPEC,
		status: "asking",
		values,
		output: outputOf({
			steps: stepsOf(4, "active", true),
			interactions: [lineFourQuestion(RUN14.started + 33 * SECOND)],
		}),
		summary: stepSummary(4),
	});
}

/** Run 13 of this session, failed at step 2 after 12 s, with the given copy. */
function run13Failed(values: FieldValues): RunEntry {
	return sessionRun({
		n: 13,
		status: "failed",
		createdAt: RUN13.created,
		startedAt: RUN13.started,
		endedAt: RUN13.started + 12 * SECOND,
		values,
		output: outputOf({
			steps: stepsOf(2, "failed", true),
			terminal: endedTerminal("failed", 12),
		}),
		outcome: failedOutcome(FAILURE_RUN_ID),
		summary: stepSummary(2),
		failedAt: stepRef(2),
	});
}

/** Run 12 of this session (compare): FL_ANSWER_EARLIER, done in 41 s. */
function run12Done(values: FieldValues): RunEntry {
	const createdAt = today("11:20:00");
	return sessionRun({
		n: 12,
		status: "done",
		createdAt,
		startedAt: createdAt + SECOND,
		endedAt: createdAt + 42 * SECOND,
		values,
		output: outputOf({
			steps: stepsOf(6, "done", false),
			answer: ANSWER_EARLIER,
			terminal: endedTerminal("completed", 41),
		}),
		outcome: SUCCEEDED,
		summary: { ...rowSummary(historyRow(12)), fileCount: 0 },
	});
}

// ─── Session state ──────────────────────────────────────────────────────────

const RAIL_DEFAULTS: Omit<RailState, "values"> = {
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

const VIEW_DEFAULTS: ViewState = {
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

const PREFS_DEFAULTS: Omit<FormPrefs, "nextRunNumber"> = {
	version: 2,
	perRun: [],
	auto: [],
	introduced: false,
	offerAnswered: false,
	noSave: [],
	forgotten: {},
	fieldSeenAt: {},
};

interface StateParts {
	readonly form: FormKey;
	readonly host?: HostCapabilities;
	readonly values?: FieldValues;
	readonly rail?: Partial<RailState>;
	readonly runs?: readonly RunEntry[];
	readonly view?: Partial<ViewState>;
	readonly prefs?: Partial<FormPrefs>;
	readonly presets?: readonly Preset[];
	readonly announcement?: Announcement | null;
	readonly undo?: UndoEntry | null;
}

const nextRunNumber = (runs: readonly RunEntry[]) =>
	runs.reduce((highest, run) => Math.max(highest, run.n), 0) + 1;

/** The highest sequence number the state hands out (focus, message, announcement, undo). */
function highestSeq(state: FormSessionState) {
	const seqs = [
		state.view.focus?.seq,
		state.view.message?.seq,
		state.announcement?.seq,
		state.undo?.seq,
	];
	return Math.max(0, ...seqs.filter((seq): seq is number => seq !== undefined));
}

function stateOf(parts: StateParts): FormSessionState {
	const host = parts.host ?? FIXTURE_APP_HOST;
	const runs = parts.runs ?? [];
	const state: FormSessionState = {
		form: fixtureForm(parts.form, host),
		layout: DESKTOP_LAYOUT,
		rail: {
			...RAIL_DEFAULTS,
			values: parts.values ?? fixtureDefaults(parts.form),
			...parts.rail,
		},
		runs,
		view: { ...VIEW_DEFAULTS, ...parts.view },
		memory: {
			loaded: true,
			prefs: {
				...PREFS_DEFAULTS,
				nextRunNumber: nextRunNumber(runs),
				...parts.prefs,
			},
			presets: parts.presets ?? [],
		},
		queue: QUEUE_BY_HOST[host.kind],
		announcement: parts.announcement ?? null,
		undo: parts.undo ?? null,
		seq: 0,
	};
	return { ...state, seq: highestSeq(state) };
}

/** The cursor in a field, caret after its value, as a click or typing leaves it. */
function focusField(seq: number, key: FieldKey): FocusRequest {
	return {
		seq,
		target: { kind: "field", key, select: false, scrollOnly: false },
	};
}

/** A dock message; one with Undo stays until the next edit or run, others for 4 s. */
function messageAt(
	seq: number,
	message: DockMessage,
	shownAt: number,
	undo = false,
): DockMessageEntry {
	return {
		seq,
		message,
		undo,
		expiresAt: undo ? null : shownAt + FORM_LIMITS.messageMs,
	};
}

function ended(
	seq: number,
	kind: "done" | "empty" | "failed" | "stopped",
	run: { n: number; seconds: number; step?: StepRef },
): Announcement {
	return {
		seq,
		kind,
		n: run.n,
		seconds: run.seconds,
		step: run.step ?? null,
	};
}

// ─── Fixtures: Extract invoice, first visit ─────────────────────────────────

function idle() {
	return stateOf({ form: "medium" });
}

/** FL_INVALID: Run pressed with the date filled; Invoice and Vendor need a look, focus on Invoice. */
function invalid() {
	return stateOf({
		form: "medium",
		values: mediumValues({ invoice_date: "2026-09-17" }),
		rail: {
			problems: {
				invoice_file: { code: "required" },
				vendor_name: { code: "required" },
			},
			pressed: true,
			lastPressAt: NOW.invalid,
		},
		view: { focus: focusField(1, "invoice_file") },
	});
}

/** Files of the first entry still sending when Run was pressed: run 1 waits in Sending. */
function uploading() {
	const pressedAt = NOW.uploading - 500;
	const sending = [sendingSlot(LIEFERSCHEIN, 0.55), sendingSlot(PO_48213, 0)];
	const values = mediumValues({
		invoice_file: sentSlot("medium", INVOICE_0917, pressedAt - 1500),
		supporting_documents: sending,
		vendor_name: NORDWIND,
		invoice_date: "2026-09-17",
		expected_total: "11769.10",
	});
	const run1 = sessionRun({
		n: 1,
		status: "sending",
		createdAt: pressedAt,
		values,
		pendingSlotIds: sending.map((slot) => slot.id),
	});
	const start = {
		n: 1,
		state: "sending",
		files: 2,
		pairs: [],
		lastFile: false,
		leftAsIs: false,
	} as const;
	return stateOf({
		form: "medium",
		values,
		runs: [run1],
		rail: { lastPressAt: pressedAt },
		view: {
			pane: "output",
			message: messageAt(1, { kind: "start", start }, pressedAt),
		},
	});
}

/** On a hosted link: both FlowPath fields show the disabled row; the dock names Invoice. */
function hostedFiles() {
	return stateOf({ form: "medium", host: FIXTURE_HOSTED_HOST });
}

// ─── Fixtures: Extract invoice with run 14 of this session ──────────────────

interface SessionExtras {
	readonly rail?: Partial<RailState>;
	readonly view?: Partial<ViewState>;
	readonly runs?: readonly RunEntry[];
	readonly presets?: readonly Preset[];
	readonly announcement?: Announcement | null;
	readonly undo?: UndoEntry | null;
	readonly values?: FieldValues;
}

/** FL_FILLED.medium in the rail, `run` of this session on the stage, older runs from history. */
function withSessionRun(
	values: FieldValues,
	run: RunEntry,
	extras: SessionExtras = {},
) {
	return stateOf({
		form: "medium",
		values: extras.values ?? values,
		runs: extras.runs ?? [run, ...mediumHistory(run.n - 1)],
		rail: { comparedRunId: run.id, lastPressAt: run.createdAt, ...extras.rail },
		view: { selectedRunId: run.id, pane: "output", ...extras.view },
		presets: extras.presets,
		announcement: extras.announcement,
		undo: extras.undo,
	});
}

function runningFixture() {
	const values = filledMedium();
	return withSessionRun(values, run14Running(values));
}

function streamingFixture() {
	const values = filledMedium();
	return withSessionRun(values, run14Streaming(values));
}

const DONE_ANNOUNCEMENT = ended(1, "done", { n: 14, seconds: 48 });

function doneFixture(extras: SessionExtras = {}) {
	const values = filledMedium();
	return withSessionRun(values, run14Done(values), {
		announcement: DONE_ANNOUNCEMENT,
		...extras,
	});
}

/** Run 13 was given what the rail holds (Main's seed), so the failure is all the screen reports. */
function failedFixture() {
	const values = filledMedium(RUN13.created - 20 * SECOND);
	return withSessionRun(values, run13Failed(values), {
		announcement: ended(1, "failed", { n: 13, seconds: 12, step: stepRef(2) }),
	});
}

function stoppedFixture() {
	const values = filledMedium();
	return withSessionRun(values, run14Stopped(values), {
		announcement: ended(1, "stopped", { n: 14, seconds: 37 }),
	});
}

/**
 * Run 12 pinned beside run 14. Both show their answers, so runs 12–14 are of this session (the
 * window has been open since before 11:20); run 13's failure was seen.
 */
function compareFixture() {
	const values = filledMedium(RUN13.created - 20 * SECOND, today("11:19:30"));
	const run14 = run14Done(values);
	const files = values.supporting_documents as readonly FileSlot[];
	const run13 = run13Failed(
		rowValues(historyRow(13), values.invoice_file as FileSlot, files),
	);
	const run12 = run12Done(
		rowValues(
			historyRow(12),
			sentSlot("medium", INVOICE_0911, today("11:19:40")),
			files,
		),
	);
	return withSessionRun(values, run14, {
		runs: [run14, run13, run12, ...mediumHistory(11)],
		view: { pinnedRunId: run12.id },
		announcement: DONE_ANNOUNCEMENT,
	});
}

/** Done with the rail on Runs; on a phone the Runs list is in the Inputs pane. */
function runsFixture() {
	return doneFixture({ rail: { tab: "runs" }, view: { pane: "inputs" } });
}

function askingFixture() {
	const values = filledMedium();
	return withSessionRun(values, run14Asking(values));
}

/** FLP_PRESETS.medium: two vendors; Nordwind applies when the form opens. */
function invoicePresets(): Preset[] {
	const createdAt = at("mon28", "09:00:00");
	return [
		{
			id: "preset-alpenfracht",
			name: "Alpenfracht AG",
			digit: 1,
			sets: { vendor_name: "Alpenfracht AG", max_pages: "60" },
			kinds: { vendor_name: "text", max_pages: "number" },
			openDefault: false,
			createdAt,
			updatedAt: createdAt,
			lastUsedAt: today("09:46:30"),
		},
		{
			id: "preset-nordwind",
			name: NORDWIND,
			digit: 2,
			sets: { vendor_name: NORDWIND },
			kinds: { vendor_name: "text" },
			openDefault: true,
			createdAt: createdAt + 60 * SECOND,
			updatedAt: createdAt + 60 * SECOND,
			lastUsedAt: RUN14.created,
		},
	];
}

/** Done with Nordwind applied on open; the Presets menu is open. */
function presetsFixture() {
	const values = filledMedium();
	return withSessionRun(values, run14Done(values, NORDWIND), {
		rail: { activePresetId: "preset-nordwind" },
		view: { overlay: { id: "presets" }, pane: "inputs" },
		presets: invoicePresets(),
		announcement: DONE_ANNOUNCEMENT,
	});
}

/** From Runs: run 9 picked and "Use these inputs" pressed; its three files must be picked again. */
function pickAgainFixture() {
	const before = filledMedium();
	const run14 = run14Done(before);
	const history = mediumHistory(13);
	const run9 = history.find((run) => run.n === 9) as RunEntry;
	const values = run9.copy.values as FieldValues;
	const inputsIn = {
		kind: "inputsIn",
		n: 9,
		pickAgain: 3,
		enterAgain: [],
		misfit: 0,
	} as const;
	return withSessionRun(before, run14, {
		values,
		runs: [run14, ...history],
		rail: { comparedRunId: run9.id, tab: "inputs" },
		view: {
			selectedRunId: run9.id,
			pane: "inputs",
			stageFollowsNewest: false,
			message: messageAt(2, inputsIn, NOW["pick-again"], true),
		},
		announcement: DONE_ANNOUNCEMENT,
		undo: {
			seq: 2,
			kind: "useInputs",
			values: before,
			nextFiles: {},
			activePresetId: null,
			deletedPreset: null,
		},
	});
}

// ─── Fixtures: the ten-invoice series (spec §6, §8) ─────────────────────────

/** Invoice, Supporting documents and Invoice date: per run for this series (`flpAutoPerRun`). */
const SERIES_AUTO: readonly string[] = [
	"invoice_file",
	"supporting_documents",
	"invoice_date",
];

const SERIES_PREFS: Partial<FormPrefs> = {
	auto: SERIES_AUTO,
	introduced: true,
};

/** Reopen: runs 1–14 on this device, none of this session; run 14 is context on the stage. */
function reopenFixture() {
	return stateOf({
		form: "medium",
		runs: mediumHistory(14),
		view: { selectedRunId: runId(14) },
	});
}

/** Right after ⌘O, ⌘A, Open: 0918 in Invoice and sending, nine next files, 0917 left out. */
function nextFilesFixture() {
	const next = [
		sendingSlot(INVOICES[1], 0.1),
		...INVOICES.slice(2).map(waitingSlot),
	];
	return stateOf({
		form: "medium",
		values: mediumValues({ invoice_file: sendingSlot(INVOICES[0], 0.4) }),
		runs: mediumHistory(14),
		rail: {
			nextFiles: { invoice_file: next },
			leftOut: { invoice_file: [{ slot: waitingSlot(PICKED_0917), n: 14 }] },
		},
		view: {
			selectedRunId: runId(14),
			list: { kind: "nextFiles", key: "invoice_file", active: -1 },
			focus: focusField(1, "invoice_file"),
		},
		prefs: SERIES_PREFS,
	});
}

/** After ↵ moved to Vendor and ↓ was pressed: the recent values list is open. */
function recentFixture() {
	const next = [
		invoiceSlot(1),
		sendingSlot(INVOICES[2], 0.5),
		sendingSlot(INVOICES[3], 0.2),
		...INVOICES.slice(4).map(waitingSlot),
	];
	return stateOf({
		form: "medium",
		values: mediumValues({ invoice_file: invoiceSlot(0) }),
		runs: mediumHistory(14),
		rail: { nextFiles: { invoice_file: next } },
		view: {
			selectedRunId: runId(14),
			list: { kind: "recent", key: "vendor_name", active: 0 },
			focus: focusField(1, "vendor_name"),
		},
		prefs: SERIES_PREFS,
	});
}

/** Run 15 was pressed at 14:31:09; the clerk presses ↵ about every 10 s. */
const SERIES_START = today("14:31:09");

/** A series run's moment: pressed, started (null while queued), ended, and how far it got. */
interface SeriesMoment {
	readonly n: number;
	readonly status: RunStatus;
	readonly pressed: number;
	readonly started: number | null;
	readonly ended: number | null;
	readonly reached: number;
}

type MomentSpec = Omit<SeriesMoment, "pressed">;

const pressedAt = (n: number) => SERIES_START + (n - 15) * 10 * SECOND;
const seriesInvoice = (n: number) => INVOICES[n - 15];
/** When each invoice of the pick (at 14:30:54) finished uploading, the current one first. */
const invoiceSentAt = (index: number) => today("14:30:59") + index * SECOND;
const invoiceSlot = (index: number) =>
	sentSlot("medium", INVOICES[index], invoiceSentAt(index));

function seriesOutput(moment: SeriesMoment): RunOutput | null {
	if (moment.started === null) return null;
	const seconds = ((moment.ended ?? moment.started) - moment.started) / SECOND;
	if (moment.status === "done")
		return outputOf({
			steps: stepsOf(STEPS.length, "done", false),
			answer: seriesAnswer(seriesInvoice(moment.n)),
			terminal: endedTerminal("completed", seconds),
		});
	if (moment.status === "failed")
		return outputOf({
			steps: stepsOf(moment.reached, "failed", false),
			terminal: endedTerminal("failed", seconds),
		});
	return outputOf({ steps: stepsOf(moment.reached, "active", false) });
}

function seriesSummary(moment: SeriesMoment): RunSummary {
	if (moment.status !== "done") return stepSummary(moment.reached);
	const number = seriesInvoice(moment.n).name.slice("invoice-".length, -4);
	return {
		...stepSummary(STEPS.length),
		firstLine: `Invoice ${number} extracted`,
		hasAnswer: true,
	};
}

/** How a series run ended: done, or failed at its step and not seen yet. */
function seriesEnding(moment: SeriesMoment): Partial<SessionRunSpec> {
	if (moment.status === "done") return { outcome: SUCCEEDED };
	if (moment.status !== "failed") return {};
	return {
		outcome: failedOutcome(backendIdOf(moment.n)),
		failedAt: stepRef(moment.reached),
		unseenFailure: true,
	};
}

/** A run of the series: defaults, Nordwind, its own invoice and date (`flpSeriesValues`). */
function seriesRun(moment: SeriesMoment): RunEntry {
	const invoice = seriesInvoice(moment.n);
	return sessionRun({
		n: moment.n,
		status: moment.status,
		createdAt: moment.pressed,
		startedAt: moment.started,
		endedAt: moment.ended,
		values: mediumValues({
			invoice_file: invoiceSlot(moment.n - 15),
			vendor_name: NORDWIND,
			invoice_date: invoice.date,
		}),
		perRun: SERIES_AUTO,
		output: seriesOutput(moment),
		summary: seriesSummary(moment),
		...seriesEnding(moment),
	});
}

const withPress = (moment: MomentSpec): SeriesMoment => ({
	...moment,
	pressed: pressedAt(moment.n),
});

/** FLP_SERIES.series: runs 15–17 going (31, 21, 11 s), 18 queued. */
const SERIES_MOMENTS: readonly SeriesMoment[] = (
	[
		{ n: 18, status: "queued", started: null, ended: null, reached: 0 },
		{
			n: 17,
			status: "running",
			started: pressedAt(17),
			ended: null,
			reached: 2,
		},
		{
			n: 16,
			status: "running",
			started: pressedAt(16),
			ended: null,
			reached: 3,
		},
		{
			n: 15,
			status: "running",
			started: pressedAt(15),
			ended: null,
			reached: 4,
		},
	] satisfies MomentSpec[]
).map(withPress);

/**
 * FLP_SERIES['series-failed'] at 14:32:32: 15–17 done (41, 48, 44 s), 18 failed at step 2 after
 * 12 s, 19–21 going (30, 25, 19 s), 22 and 23 queued. Each queued run started when a place freed.
 */
const SERIES_FAILED_MOMENTS: readonly SeriesMoment[] = (
	[
		{ n: 23, status: "queued", started: null, ended: null, reached: 0 },
		{ n: 22, status: "queued", started: null, ended: null, reached: 0 },
		{
			n: 21,
			status: "running",
			started: today("14:32:13"),
			ended: null,
			reached: 3,
		},
		{
			n: 20,
			status: "running",
			started: today("14:32:07"),
			ended: null,
			reached: 4,
		},
		{
			n: 19,
			status: "running",
			started: today("14:32:02"),
			ended: null,
			reached: 4,
		},
		{
			n: 18,
			status: "failed",
			started: today("14:31:50"),
			ended: today("14:32:02"),
			reached: 2,
		},
		{
			n: 17,
			status: "done",
			started: pressedAt(17),
			ended: today("14:32:13"),
			reached: 6,
		},
		{
			n: 16,
			status: "done",
			started: pressedAt(16),
			ended: today("14:32:07"),
			reached: 6,
		},
		{
			n: 15,
			status: "done",
			started: pressedAt(15),
			ended: today("14:31:50"),
			reached: 6,
		},
	] satisfies MomentSpec[]
).map(withPress);

const RUN18_QUEUED = {
	n: 18,
	state: "queued",
	files: 1,
	pairs: ["invoice-RE-2026-0921.pdf", "22 Sep 2026"],
	lastFile: false,
	leftAsIs: false,
} as const;

interface SeriesOverrides {
	readonly view?: Partial<ViewState>;
	readonly rail?: Partial<RailState>;
}

/**
 * The cursor the session places after a run (reduce-press placeCursor): in the field on a split box,
 * the field only scrolled into view on a phone, so no keyboard opens (spec §8.12 "not focused").
 */
function afterRunCursor(seq: number, key: FieldKey): FocusRequest {
	return {
		seq,
		target: { kind: "field", key, select: true, scrollOnly: true },
	};
}

/**
 * Right after the fourth ↵ (14:31:39): run 18 queued, its start message showing for 4 s; 0922 in
 * Invoice with 0923–0927 next, Vendor kept, Invoice date empty and focused (on a split box).
 */
function seriesFixture(name: FixtureName, overrides: SeriesOverrides = {}) {
	const now = NOW[name];
	const fresh = now < pressedAt(18) + FORM_LIMITS.messageMs;
	const start = messageAt(
		2,
		{ kind: "start", start: RUN18_QUEUED },
		pressedAt(18),
	);
	return stateOf({
		form: "medium",
		values: mediumValues({
			invoice_file: invoiceSlot(4),
			vendor_name: NORDWIND,
		}),
		runs: [...SERIES_MOMENTS.map(seriesRun), ...mediumHistory(14)],
		rail: {
			nextFiles: {
				invoice_file: INVOICES.slice(5).map((_, offset) =>
					invoiceSlot(5 + offset),
				),
			},
			dateAnchors: { invoice_date: "2026-09-22" },
			comparedRunId: runId(17),
			lastPressAt: pressedAt(18),
			...overrides.rail,
		},
		view: {
			selectedRunId: runId(17),
			focus: afterRunCursor(1, "invoice_date"),
			message: fresh ? start : null,
			...overrides.view,
		},
		prefs: SERIES_PREFS,
	});
}

/** A dialog opened from the keyboard (⌘/, ⌘S): the key press in the rail ended the start message. */
function seriesWithOverlay(name: FixtureName, overlay: Overlay) {
	return seriesFixture(name, {
		view: { overlay, focus: null, message: null },
	});
}

/** The tenth file is in and "30" is typed; run 18's failure is unseen behind the overflow. */
function seriesFailedFixture() {
	return stateOf({
		form: "medium",
		values: mediumValues({
			invoice_file: invoiceSlot(9),
			vendor_name: NORDWIND,
		}),
		runs: [...SERIES_FAILED_MOMENTS.map(seriesRun), ...mediumHistory(14)],
		rail: {
			texts: { invoice_date: "30" },
			dateAnchors: { invoice_date: "2026-09-29" },
			comparedRunId: runId(21),
			lastPressAt: pressedAt(23),
		},
		view: {
			selectedRunId: runId(21),
			focus: focusField(1, "invoice_date"),
		},
		prefs: SERIES_PREFS,
		announcement: ended(2, "done", { n: 17, seconds: 44 }),
	});
}

/** Series after a click on the Run 18 tab (the start message is gone): the queued run on the stage. */
function queuedFixture() {
	return seriesFixture("queued", {
		rail: { comparedRunId: runId(18) },
		view: {
			selectedRunId: runId(18),
			pane: "output",
			focus: null,
			stageFollowsNewest: false,
		},
	});
}

// ─── Fixtures: Return request ───────────────────────────────────────────────

function parcelValues(index: number, sentAt: number): FieldValues {
	const parcel = PARCELS[index];
	return {
		order: parcel.order,
		quantity: parcel.quantity,
		receipt: sentSlot("small", parcel.receipt, sentAt),
	};
}

function smallDone() {
	const createdAt = today("10:12:00");
	const values = parcelValues(0, createdAt - 5 * SECOND);
	const run1 = sessionRun({
		n: 1,
		status: "done",
		createdAt,
		startedAt: createdAt + SECOND,
		endedAt: createdAt + 4 * SECOND,
		values,
		output: outputOf({
			result: { value: FIXTURE_RESULT_SMALL },
			terminal: endedTerminal("completed", 3),
		}),
		outcome: SUCCEEDED,
		summary: {
			...SUMMARY_BASE,
			firstLine: "Return number RET-20931",
			hasResult: true,
		},
	});
	return stateOf({
		form: "small",
		values,
		runs: [run1],
		rail: { comparedRunId: run1.id, lastPressAt: createdAt },
		view: { selectedRunId: run1.id, pane: "output" },
		announcement: ended(1, "done", { n: 1, seconds: 3 }),
	});
}

/** A Return request run that returned FL_RESULT_SMALL after 3 s. */
function returned(createdAt: number): Partial<SessionRunSpec> {
	return {
		status: "done",
		endedAt: createdAt + 3 * SECOND,
		output: outputOf({
			result: { value: FIXTURE_RESULT_SMALL },
			terminal: endedTerminal("completed", 3),
		}),
		outcome: SUCCEEDED,
		summary: {
			...SUMMARY_BASE,
			firstLine: "Return number RET-20931",
			hasResult: true,
		},
	};
}

/** `flpParcelRuns()`: run 3 just started, runs 1 and 2 took 3 s; each order number was typed over. */
function parcelRuns(): RunEntry[] {
	const runs = PARCELS.map((_, index) => {
		const createdAt = today("10:14:20") + index * 30 * SECOND;
		const last = index === PARCELS.length - 1;
		return sessionRun({
			n: index + 1,
			status: "running",
			createdAt,
			startedAt: createdAt,
			values: parcelValues(index, createdAt - 3 * SECOND),
			replaced: ["order"],
			output: outputOf({}),
			...(last ? {} : returned(createdAt)),
		});
	});
	return runs.reverse();
}

/** As the third parcel's run starts, the offer "Make Order number and Receipt per run?". */
function smallOffer() {
	const runs = parcelRuns();
	const run3 = runs[0];
	return stateOf({
		form: "small",
		values: run3.copy.values as FieldValues,
		runs,
		rail: { comparedRunId: run3.id, lastPressAt: run3.createdAt },
		view: {
			selectedRunId: run3.id,
			pane: "output",
			question: { kind: "offer", names: ["order", "receipt"] },
		},
		announcement: ended(1, "done", { n: 2, seconds: 3 }),
	});
}

/** The same moment after "Yes": both fields per run and empty, the cursor in Order number. */
function smallReset() {
	const runs = parcelRuns();
	const run3 = runs[0];
	const now = NOW["small-reset"];
	return stateOf({
		form: "small",
		values: fixtureDefaults("small"),
		runs,
		rail: { comparedRunId: run3.id, lastPressAt: run3.createdAt },
		view: {
			selectedRunId: run3.id,
			focus: focusField(2, "order"),
			message: messageAt(
				3,
				{ kind: "perRunNow", labels: ["Order number", "Receipt"] },
				now,
			),
		},
		prefs: {
			perRun: ["order", "receipt"],
			introduced: true,
			offerAnswered: true,
		},
		announcement: ended(1, "done", { n: 2, seconds: 3 }),
	});
}

// ─── Fixtures: Triage selected request (no fields) ──────────────────────────

function noneDone() {
	const createdAt = today("09:10:00");
	const run1 = sessionRun({
		n: 1,
		status: "empty",
		createdAt,
		startedAt: createdAt + 500,
		endedAt: createdAt + 2500,
		values: {},
		output: outputOf({ terminal: endedTerminal("completed", 2) }),
		outcome: SUCCEEDED,
	});
	return stateOf({
		form: "none",
		runs: [run1],
		rail: { lastPressAt: createdAt },
		view: { selectedRunId: run1.id, pane: "output" },
		announcement: ended(1, "empty", { n: 1, seconds: 2 }),
	});
}

/** `flpQuickRuns()`: three runs going, a fourth press refused (a form without fields never queues). */
function noneCap() {
	const now = NOW["none-cap"];
	const runs = [
		{ n: 3, pressed: today("09:12:05"), started: today("09:12:05") },
		{ n: 2, pressed: today("09:12:04"), started: today("09:12:04") + 100 },
		{ n: 1, pressed: today("09:12:03"), started: today("09:12:03") + 400 },
	].map((run) =>
		sessionRun({
			n: run.n,
			status: "running",
			createdAt: run.pressed,
			startedAt: run.started,
			values: {},
			output: outputOf({}),
		}),
	);
	return stateOf({
		form: "none",
		runs,
		rail: { lastPressAt: now },
		view: {
			selectedRunId: runId(3),
			pane: "output",
			focus: { seq: 1, target: { kind: "runAgain" } },
		},
		announcement: { seq: 2, kind: "capReached", running: 3 },
	});
}

// ─── Fixtures: Supplier portal quality run ──────────────────────────────────

/** FL_FILLED.large typed in, nothing run. */
function large() {
	return stateOf({
		form: "large",
		values: {
			...fixtureDefaults("large"),
			portal_url: "https://portal.nordwind-logistik.example/login",
			customer_number: "K-204418",
			order_numbers: ["PO-48190", "PO-48213", "PO-48231", "PO-48240"],
			date_from: "2026-09-01",
			report_title: "Supplier portal check, September",
		},
	});
}

// ─── The table ──────────────────────────────────────────────────────────────

const FIXTURES: Readonly<Record<FixtureName, () => FormSessionState>> = {
	idle,
	invalid,
	running: runningFixture,
	streaming: streamingFixture,
	done: () => doneFixture(),
	failed: failedFixture,
	stopped: stoppedFixture,
	none: () => stateOf({ form: "none" }),
	"none-done": noneDone,
	small: () => stateOf({ form: "small" }),
	"small-done": smallDone,
	large,
	compare: compareFixture,
	runs: runsFixture,
	uploading,
	reopen: reopenFixture,
	"next-files": nextFilesFixture,
	recent: recentFixture,
	series: () => seriesFixture("series"),
	"series-failed": seriesFailedFixture,
	queued: queuedFixture,
	"after-run": () =>
		seriesFixture("after-run", {
			view: { overlay: { id: "afterRun", focusName: null }, focus: null },
		}),
	"small-offer": smallOffer,
	"small-reset": smallReset,
	"none-cap": noneCap,
	"hosted-files": hostedFiles,
	shortcuts: () => seriesWithOverlay("shortcuts", { id: "shortcuts" }),
	presets: presetsFixture,
	"preset-save": () =>
		seriesWithOverlay("preset-save", {
			id: "presetSave",
			mode: "save",
			fromRunId: null,
		}),
	"pick-again": pickAgainFixture,
	asking: askingFixture,
	leave: () =>
		seriesFixture("leave", {
			view: {
				overlay: { id: "leave", route: "/", replace: false },
				focus: null,
			},
		}),
};

/** A fresh session state for one artboard or spec preset, with the desktop layout. */
export function fixture(name: FixtureName): FormSessionState {
	return FIXTURES[name]();
}

// ─── What each fixture is for ───────────────────────────────────────────────

export interface FixtureArtboard {
	/** The canvas wrapper (`SP/canvas-a/project/<file>`). */
	readonly file: string;
	readonly device: ArtboardDevice;
	readonly theme: "light" | "dark";
	/** `SP/shots/a/<render>`; null for spec §8 artboards, which have no render. */
	readonly render: string | null;
}

export interface FixtureMeta {
	readonly name: FixtureName;
	readonly form: FormKey;
	readonly title: string;
	/** The app's name, for the host header. */
	readonly appName: string;
	readonly routeLabels: Readonly<Record<string, string>>;
	readonly artboards: readonly FixtureArtboard[];
}

const board = (
	file: string,
	render: string | null,
	device: ArtboardDevice = "desktop",
	theme: "light" | "dark" = "light",
): FixtureArtboard => ({ file, render, device, theme });

const META: Readonly<
	Record<FixtureName, readonly [FormKey, string, readonly FixtureArtboard[]]>
> = {
	idle: [
		"medium",
		"First visit · nine fields",
		[
			board("Main.dc.html", "pub3-Main.png"),
			board("Phone.dc.html", "pub3-Phone.png", "phone"),
		],
	],
	invalid: [
		"medium",
		"Run pressed · two fields need a look",
		[board("Invalid.dc.html", "pub3-Invalid.png")],
	],
	running: [
		"medium",
		"Running · step 4",
		[
			board("Running.dc.html", "pub3-Running.png"),
			board("PhoneRunning.dc.html", "pub3-PhoneRunning.png", "phone"),
		],
	],
	streaming: [
		"medium",
		"The answer streams in",
		[board("Streaming.dc.html", "pub3-Streaming.png")],
	],
	done: [
		"medium",
		"Done · answer, result, 12 files",
		[
			board("Done.dc.html", "pub3-Done.png"),
			board("Dark.dc.html", "pub3-Dark.png", "desktop", "dark"),
			board("PhoneDone.dc.html", "pub3-PhoneDone.png", "phone"),
		],
	],
	failed: [
		"medium",
		"Failed at step 2",
		[board("Failed.dc.html", "pub3-Failed.png")],
	],
	stopped: [
		"medium",
		"Stopped at 0:37",
		[board("Stopped.dc.html", "pub3-Stopped.png")],
	],
	none: [
		"none",
		"No fields · before the first run",
		[
			board("NoFields.dc.html", "pub3-NoFields.png"),
			board("PhoneNoFields.dc.html", "pub3-PhoneNoFields.png", "phone"),
		],
	],
	"none-done": [
		"none",
		"No fields · finished, nothing returned",
		[board("NoFieldsDone.dc.html", "pub3-NoFieldsDone.png")],
	],
	small: ["small", "Three fields", [board("Small.dc.html", "pub3-Small.png")]],
	"small-done": [
		"small",
		"Three fields · a small result",
		[board("SmallDone.dc.html", "pub3-SmallDone.png")],
	],
	large: ["large", "29 fields", [board("Large.dc.html", "pub3-Large.png")]],
	compare: [
		"medium",
		"Two runs side by side",
		[board("Compare.dc.html", "pub3-Compare.png")],
	],
	runs: [
		"medium",
		"14 runs on this device",
		[board("Runs.dc.html", "pub3-Runs.png")],
	],
	uploading: ["medium", "Run pressed while files send", []],
	reopen: ["medium", "Opened again · 14 runs", [board("Reopen.dc.html", null)]],
	"next-files": [
		"medium",
		"Ten invoices picked at once",
		[board("NextFiles.dc.html", null)],
	],
	recent: [
		"medium",
		"Recent values for Vendor",
		[board("Recent.dc.html", null)],
	],
	series: [
		"medium",
		"One run per invoice",
		[
			board("Series.dc.html", null),
			board("PhoneSeries.dc.html", null, "phone"),
			board("SeriesDark.dc.html", null, "desktop", "dark"),
		],
	],
	"series-failed": [
		"medium",
		"Last invoice · a failure surfaced",
		[board("SeriesFailed.dc.html", null)],
	],
	queued: ["medium", "A queued run", [board("Queued.dc.html", null)]],
	"after-run": ["medium", "Per-run inputs", [board("AfterRun.dc.html", null)]],
	"small-offer": [
		"small",
		"Three fields · the offer",
		[board("SmallOffer.dc.html", null)],
	],
	"small-reset": [
		"small",
		"Three fields · next parcel",
		[board("SmallReset.dc.html", null)],
	],
	"none-cap": [
		"none",
		"Quick action · three runs going",
		[board("NoFieldsCap.dc.html", null)],
	],
	"hosted-files": [
		"medium",
		"Hosted link · files can't be sent yet",
		[board("HostedFiles.dc.html", null)],
	],
	shortcuts: [
		"medium",
		"Keyboard shortcuts",
		[board("Shortcuts.dc.html", null)],
	],
	presets: [
		"medium",
		"Presets · two vendors",
		[board("Presets.dc.html", null)],
	],
	"preset-save": [
		"medium",
		"Save inputs as a preset",
		[board("PresetSave.dc.html", null)],
	],
	"pick-again": [
		"medium",
		"An older run's files",
		[board("PickAgain.dc.html", null)],
	],
	asking: ["medium", "Waiting for your answer · in-run question", []],
	leave: ["medium", "Leave this form? · series", []],
};

/** The form, title, host header text and artboards (with their renders) of a fixture. */
export function fixtureMeta(name: FixtureName): FixtureMeta {
	const [form, title, artboards] = META[name];
	return {
		name,
		form,
		title,
		appName: FORM_SPECS[form].appName,
		routeLabels: fixtureRouteLabels(form),
		artboards,
	};
}
