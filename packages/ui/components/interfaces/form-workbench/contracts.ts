/*
 * Contracts of the form workbench: direction A · Workbench plus the power-user layer.
 * Plan: todo/form-workbench/PLAN.md. Spec: SPEC-A-POWER.md revision of 2026-10-05 11:51 (hash in PLAN §0).
 *
 * Types and constant tables only. Lanes never edit this file. A lane that needs a change
 * writes `CONTRACT-REQUEST:` in its notes; the architect applies changes between waves.
 */
import type { ReactNode, RefObject } from "react";
import type { IIntercomEvent } from "../../../lib/schema/events/intercom-event";
import type { IEvent, IEventInput } from "../../../lib/schema/flow/event";
import type { IEventPayload } from "../../../lib/schema/flow/event-payload";
import type { ILogMetadata } from "../../../lib/schema/flow/log-metadata";
import type { IInteractionRequest } from "../../../lib/schema/interaction";
import type {
	IHelperState,
	ITemporaryUploadExecutionTarget,
} from "../../../state/backend-state/helper-state";
import type { Tone } from "../../settings/devices/primitives/tone";
import type { IAttachment } from "../chat-default/chat-db";
import type { IToolBarActions, IUseInterfaceProps } from "../interfaces";

// ─── Hosts ──────────────────────────────────────────────────────────────────

/** Where the form runs. `app`: desktop and web `/use`, Home tiles, lesson pane. `hosted`: public `/a` link. `service`: a device's service page (standalone `/ui/` and the Devices runtime view). */
export type FormHostKind = "app" | "hosted" | "service";
/** `page` may take the cursor on open (S6); `tile` never does. */
export type FormPresentation = "page" | "tile";
/** `device`: IndexedDB `flow-like-form-memory`, keyed by `memoryScope`. `session`: memory for this page session only. */
export type FormPersistence = "device" | "session";
/** Where a run executes: decides the upload's FlowPath source and the cap (spec F, M5). */
export type ExecutionTarget = ITemporaryUploadExecutionTarget;

/** Props of `FormWorkbenchInterface` (form-workbench/index.tsx). `presentation` comes from `IUseInterfaceProps`. */
export interface FormWorkbenchProps extends IUseInterfaceProps {
	/** Absent means `app`, except for a Devices runtime namespace (`device-runtime:`), which is `service`. */
	readonly host?: FormHostKind;
}

/**
 * Decided from the host kind first, never from which helper methods exist: the hosted helper
 * inherits a throwing `fileToTemporaryFile` from EmptyHelperState.
 */
export interface HostCapabilities {
	readonly kind: FormHostKind;
	readonly presentation: FormPresentation;
	/** `device` only on the app host with a memory scope (desktop profile, signed-in web user). */
	readonly persistence: FormPersistence;
	/** `profile:<id>` on desktop, `user:<sub>` on the signed-in web; null with session persistence. */
	readonly memoryScope: string | null;
	/** `temporary`: app host with `filesToTemporaryFiles`. `inline`: hosted and service, data: URLs encoded when the run is sent. */
	readonly uploads: "temporary" | "inline";
	/** A FlowPath field can be filled: app host, temporary uploads, signed in on the web. */
	readonly flowPathFiles: boolean;
	/** A one-file field takes several picked files (M3): app and service, never hosted. */
	readonly nextFiles: boolean;
	/** `cancel`: eventState.cancelExecution stops the run. `detach`: it only stops listening (hosted). */
	readonly stop: "cancel" | "detach";
	/** Inline hosts: the most one file may weigh (service: MAX_ATTACHMENT_BYTES, never above the room). */
	readonly inlineFileLimitBytes: number | null;
	/** Inline hosts: bytes of files one run can carry, (request − reserve) × 3/4 (`flpRoom`). */
	readonly inlineRoomBytes: number | null;
	/** Upload hosts warn above this size (35 MB); null elsewhere. */
	readonly warnFileBytes: number | null;
	/** `remote` on hosted links, `local` on service pages; null on the app host, where it is resolved per session. */
	readonly fixedTarget: ExecutionTarget | null;
	/** The host passed `toolbarRef` (prop presence, never `toolbarRef.current`). */
	readonly hasToolbar: boolean;
}

export interface HostCapabilityInput {
	readonly kind: FormHostKind;
	readonly presentation: FormPresentation;
	readonly helperState: Pick<
		IHelperState,
		"fileToUrl" | "fileToTemporaryFile" | "filesToTemporaryFiles"
	>;
	readonly hasToolbar: boolean;
	/** Desktop counts as signed in; on the web, AuthContext says so. */
	readonly signedIn: boolean;
	/** See `HostCapabilities.memoryScope`; null when there is none. */
	readonly memoryScope: string | null;
}
export type ResolveHostCapabilities = (
	input: HostCapabilityInput,
) => HostCapabilities;

/** The viewer's date habits from Intl and `navigator.language`, never from the app language (spec M4). */
export interface DateLocale {
	readonly order: "dmy" | "mdy" | "ymd";
	/** Separator of numeric dates, for examples and copy ("/" or "."). */
	readonly sep: string;
	/** Per month (index 0 = January): folded names in the viewer's language and English. */
	readonly months: readonly (readonly string[])[];
	readonly words: {
		readonly today: readonly string[];
		readonly yesterday: readonly string[];
		readonly tomorrow: readonly string[];
	};
}

export interface ViewerHabits {
	/** ⌘ on macOS, Ctrl elsewhere, in titles, the sheet and copy. */
	readonly mac: boolean;
	/** `navigator.language`, for Intl formatting of dates, times and sizes. */
	readonly locale: string;
	readonly dateLocale: DateLocale;
	/** From `Intl.NumberFormat(locale).formatToParts(1.1)` (S3). */
	readonly decimalSign: "." | ",";
}
export interface ViewerInput {
	readonly platform: string;
	readonly language: string;
}
export type ResolveViewerHabits = (input: ViewerInput) => ViewerHabits;

// ─── Layout ─────────────────────────────────────────────────────────────────

/** Measured by the shell on the interface root (ResizeObserver), never from the viewport. */
export interface WorkbenchLayout {
	readonly width: number;
	readonly height: number;
	/** Box at least LAYOUT.splitMinWidth: rail and stage side by side. */
	readonly split: boolean;
	/** `(pointer: coarse)`: 44 px targets and 16 px field text. */
	readonly touch: boolean;
	/** `(pointer: fine)`: typed dates with the calendar, recent values, the keyboard icon, S6. */
	readonly finePointer: boolean;
	/** Stage at least LAYOUT.compareMinStageWidth: two runs side by side. */
	readonly compare: boolean;
}

export const LAYOUT = {
	splitMinWidth: 900,
	railWidth: 400,
	compareMinStageWidth: 600,
	phoneDockHeight: 56,
	stripHeight: 40,
	stripTabMinWidth: 110,
	stripOverflowReserve: 110,
	stripOverflowReserveWithFailure: 180,
	touchTarget: 44,
	answerMaxWidthRem: 38,
} as const;

// ─── Limits and names ───────────────────────────────────────────────────────

/**
 * Sizes in bytes; MB is 1,048,576 bytes. Mirrors SPEC-A-POWER FLP_LIMITS and FLP_HOSTS. The
 * service page's file and request limits are not here: session/host.ts imports MAX_ATTACHMENT_BYTES
 * and MAX_REQUEST_BYTES from lib/service-runtime/backend.ts.
 */
export const FORM_LIMITS = {
	/** Runs on this device at once from this window. */
	localParallel: 3,
	/** Runs in the cloud at once until the tier's max_concurrent_executions is known. */
	unknownTierParallel: 2,
	/** A start refused with concurrent_cloud_executions is tried again after this, or when a run of this window ends. */
	retryMs: 15_000,
	doublePressMs: 700,
	nextFiles: 50,
	nextFilesShown: 6,
	uploadsAhead: 2,
	recentRuns: 50,
	recentShown: 6,
	recentMaxChars: 200,
	offerMaxChars: 40,
	pairingMaxChars: 40,
	messageMs: 4000,
	pastDaysWarn: 14,
	leaveGraceMs: 2000,
	historyPerForm: 200,
	presetsPerForm: 30,
	presetDigits: 9,
	presetNameChars: 60,
	presetBytes: 16 * 1024,
	presetFindFrom: 6,
	prefsPruneMs: 90 * 24 * 60 * 60 * 1000,
	warnFileBytes: 35 * 1024 * 1024,
	/** axum's default body limit on /a/{app_id}/invoke. */
	hostedRequestBytes: 2 * 1024 * 1024,
	hostedReserveBytes: 64 * 1024,
	serviceReserveBytes: 512 * 1024,
	filterFromFields: 12,
	presetsFromFields: 12,
	presetsFromRuns: 2,
	offerFromRuns: 3,
	autofocusFromRuns: 2,
	shortcutsIconFromFields: 4,
	slowRunNoteMs: 10_000,
} as const;

export const MEMORY_DB_NAME = "flow-like-form-memory";
/** Same as flow_like_types::dispatch::REQUEST_FILES_STORE_REF. */
export const REQUEST_FILES_STORE_REF = "__flow_like_http_request_files";
/** Inputs with this name (any case) are the catalog's catch-all pin and never shown. */
export const PAYLOAD_PIN_NAME = "payload";
/** What secret fields and secret-looking values show in chips, Compare, Inputs, the Runs list and start messages. */
export const SECRET_MASK = "••••";
/** Joins an object field's name and a property name into a FieldKey; it cannot occur in pin names. */
export const FIELD_KEY_SEPARATOR = "\u001f";

/**
 * Focus contract. Every element the session may focus carries `data-fw-focus` with one of
 * these values; the shell finds it inside the interface root (values pass through CSS.escape).
 * Field values are `field:<FieldKey>`, run tabs `tab:<runId>`.
 */
export const FOCUS_ATTR = "data-fw-focus";
export const FOCUS_VALUE = {
	run: "run",
	runAgain: "run-again",
	stop: "stop",
	change: "change",
	copy: "copy",
	presetButton: "preset-button",
	filter: "filter",
	fieldPrefix: "field:",
	tabPrefix: "tab:",
} as const;
/** On the interface root until the first key press (S6): the shell's scoped style hides every focus ring. */
export const QUIET_FOCUS_ATTR = "data-fw-quiet-focus";

/**
 * File fields mark their container `data-fw-file-field="<name>"` (plus `data-empty` while empty) and their
 * hidden `<input type="file">` `data-fw-file-input="<name>"`; the shell's ⌘O clicks that input.
 */
export const FILE_FIELD_ATTR = "data-fw-file-field";
export const FILE_INPUT_ATTR = "data-fw-file-input";

/** i18n: namespace `interfaces`, every key under `workbench.<lane area>.`; literal keys only (PLAN §9). */
export const I18N_NAMESPACE = "interfaces";
export const I18N_PREFIX = "workbench";

// ─── Event inputs ───────────────────────────────────────────────────────────

/**
 * IEventInput plus what spec 10.1 adds. Lane B-INPUTS adds the same optional fields to IEventInput;
 * until then every event reads as if none were delivered. `range` is `[min, max]` as serde writes it.
 */
export type WorkbenchEventInput = IEventInput & {
	readonly sensitive?: boolean;
	readonly valid_values?: readonly string[] | null;
	readonly range?: readonly [number, number] | null;
	readonly step?: number | null;
	/** A sensitive pin's default exists but is withheld; the server fills it when the input is left out. */
	readonly default_omitted?: boolean;
};

// ─── Fields ─────────────────────────────────────────────────────────────────

/**
 * The control a field gets. `file`/`files` cover FlowPath structs (Normal / Array or HashSet)
 * and legacy PathBuf and Byte pins. `pairs`: HashMap of scalars, or a Struct without a usable
 * schema (also one whose schema is still an unresolved all-digit ref key). `json`: Generic,
 * Geometry and nested shapes, a text box parsed as JSON.
 */
export type FieldKind =
	| "text"
	| "number"
	| "bool"
	| "date"
	| "choice"
	| "chips"
	| "file"
	| "files"
	| "group"
	| "pairs"
	| "json"
	| "unsupported";

/** `flowpath`: the run gets FlowPath objects. `url`: legacy PathBuf/Byte, the run gets URL strings. */
export type FileMode = "flowpath" | "url";

/** A field's name, or `<object name>${FIELD_KEY_SEPARATOR}<property>`; built and resolved only through the field list. */
export type FieldKey = string;

export interface WorkbenchField {
	readonly key: FieldKey;
	/** Payload key: the pin name, or the property name inside its group. */
	readonly name: string;
	readonly label: string;
	/** Resolved description; null when empty or still an unresolved all-digit ref key. */
	readonly help: string | null;
	readonly kind: FieldKind;
	readonly dataType: string;
	readonly valueType: string;
	/** `!optional`, except switches, groups and pairs, which are never required as a whole. */
	readonly required: boolean;
	/** The pin's own `optional` flag ("Optional" on an optional object or map); `pinOptional` falls back to `!required`. */
	readonly optional?: boolean;
	/** Delivered by the backend (`sensitive`): masked input with Show / Hide. Guessed secrets are model/secrets.ts. */
	readonly sensitive: boolean;
	/** `default_omitted`: optional → empty and left out; required → "The app's own value is not shown here." */
	readonly defaultOmitted: boolean;
	/**
	 * The starting value without a preset: the stored default, else for an optional field the type
	 * default as today's form seeds it (a date is today, a number 0, a switch off), else empty.
	 * Empty for files and for withheld sensitive defaults.
	 */
	readonly defaultValue: FieldValue;
	/** A stored default exists or the optional field was seeded with a non-empty type default (dot rule, spec 2.0). */
	readonly hasDefault: boolean;
	/** `valid_values` of a String, Integer, Float or Date field: a choice; the typed value is sent. */
	readonly options: readonly string[] | null;
	readonly range: readonly [number, number] | null;
	readonly step: number | null;
	readonly integer: boolean;
	readonly fileMode: FileMode | null;
	/** Member control of a `chips` field. */
	readonly itemKind: "text" | "number" | "date" | null;
	/** Object properties with `format: date` send `YYYY-MM-DD`; `date-time` sends RFC 3339. */
	readonly dateFormat: "dateTime" | "date" | null;
	/** Properties of a `group` field, in schema order. */
	readonly props: readonly WorkbenchField[];
	/** Numbers, dates, switches and short choices may sit two per row on a wide rail. */
	readonly short: boolean;
	readonly index: number;
}

export type FieldsFromEvent = (
	inputs: readonly WorkbenchEventInput[],
) => readonly WorkbenchField[];

// ─── Values ─────────────────────────────────────────────────────────────────

export interface FlowPath {
	readonly path: string;
	readonly store_ref: string;
	readonly cache_store_ref: string | null;
}

/** What a sent file stands for in a run's payload. `inline`: encoded with fileToUrl when its run is sent. */
export type FileRef =
	| {
			readonly kind: "flowpath";
			readonly flowPath: FlowPath;
			readonly url: string | null;
	  }
	| { readonly kind: "url"; readonly url: string }
	| { readonly kind: "inline" };

/** `reminder`: a file of an older run that must be picked again (S4); it is not a value. */
export type FileSlotState =
	| "waiting"
	| "sending"
	| "sent"
	| "failed"
	| "reminder";

/**
 * One file in a field, a next-files list, a left-out note or a run's copy. The File object lives in
 * the session runtime, keyed by `id`; the reducer updates a slot by id wherever it appears.
 */
export interface FileSlot {
	readonly id: string;
	readonly name: string;
	readonly size: number | null;
	readonly type: string | null;
	readonly state: FileSlotState;
	/** 0–1 while sending. */
	readonly progress: number | null;
	readonly ref: FileRef | null;
	readonly error: string | null;
	/** When the upload ended (ms epoch). */
	readonly sentAt: number | null;
	/** `downloadExpiresAt` of a URL upload (ms epoch): it cannot be sent again afterwards (S4). */
	readonly expiresAt: number | null;
}

export interface PickedFile {
	readonly slotId: string;
	readonly name: string;
	readonly size: number;
	readonly type: string;
}

/** A picked file a run on this device was already given in this field (M3), with that run's number. */
export interface LeftOutFile {
	readonly slot: FileSlot;
	readonly n: number;
}

export interface PairRow {
	readonly id: string;
	readonly key: string;
	readonly value: string;
}

export interface GroupValue {
	readonly [property: string]: FieldValue;
}

/**
 * A rail value, narrowed by the field's kind:
 * text, number (as typed), date (`YYYY-MM-DD` or ""), choice, json → string ·
 * bool → boolean · chips → string[] · file → FileSlot | null · files → FileSlot[] ·
 * group → GroupValue · pairs → PairRow[].
 */
export type FieldValue =
	| string
	| boolean
	| readonly string[]
	| FileSlot
	| readonly FileSlot[]
	| GroupValue
	| readonly PairRow[]
	| null;

export type FieldValues = Readonly<Record<string, FieldValue>>;

/** A value kept out of storage: a secret field or a secret-looking value (M6). Tagged so no GroupValue can look like it. */
export interface HiddenValue {
	readonly $hidden: true;
}
export type CopyValue = FieldValue | HiddenValue;

export type FieldProblemCode =
	| "required"
	| "integer"
	| "number"
	| "noDecimals"
	| "range"
	| "date"
	| "json"
	| "object"
	| "array"
	| "unique"
	| "items"
	| "option"
	| "fileSending"
	| "fileFailed"
	| "fileTooLarge"
	| "fileNotHere"
	| "pickAgain"
	| "enterAgain"
	| "unsupported";

export interface FieldProblem {
	readonly code: FieldProblemCode;
	readonly fileName?: string;
	readonly min?: number;
	readonly max?: number;
	readonly limitBytes?: number;
}

/** A run's input value as the run sends it. */
export type PayloadResult =
	| { readonly ok: true; readonly payload: Readonly<Record<string, unknown>> }
	| {
			readonly ok: false;
			readonly problems: Readonly<Record<FieldKey, FieldProblem>>;
	  };

/**
 * Switches are always sent. An empty optional field is left out (the server fills its default).
 * A FlowPath field sends FlowPath objects and never a URL; a slot without one is a problem.
 */
export type BuildPayload = (
	fields: readonly WorkbenchField[],
	values: Readonly<Record<string, CopyValue>>,
) => PayloadResult;

/** One input that differs between two value sets (change chips, 2 px edges, Compare table). Secrets read SECRET_MASK. */
export interface FieldChange {
	readonly name: string;
	readonly label: string;
	readonly from: string;
	readonly to: string;
}

/** Words the pure helpers need for short value text; UI lanes build it with `t` and Intl (model/date-text.ts). */
export interface ShortWords {
	readonly none: string;
	readonly empty: string;
	readonly on: string;
	readonly off: string;
	readonly files: (count: number) => string;
	readonly entries: (count: number) => string;
	/** `YYYY-MM-DD` → "18 Sep 2026" in the viewer's language. */
	readonly date: (iso: string) => string;
}

// ─── Typed dates (M4) ───────────────────────────────────────────────────────

/** `past`: days before the anchor when a day alone reads more than FORM_LIMITS.pastDaysWarn days back, else 0. The UI formats the reading. */
export interface DateReading {
	readonly iso: string;
	readonly past: number;
}
/** null: the text cannot be read. `{ iso: "", past: 0 }`: empty text. */
export type ReadDate = (
	text: string,
	anchorIso: string | null,
	todayIso: string,
	locale: DateLocale,
) => DateReading | null;

// ─── Keyboard (M2) ──────────────────────────────────────────────────────────

/** The next empty required stop after `fromKey`, wrapping; `skip` names fields that are no stop (blocked); null means "run now". */
export type EnterTarget = (
	fields: readonly WorkbenchField[],
	values: FieldValues,
	fromKey: FieldKey,
	skip: readonly string[],
) => FieldKey | null;

/** ↵ never starts a run whose inputs equal the newest run of this session (`newest`: its copy, or null). */
export type EnterMayRun = (
	fields: readonly WorkbenchField[],
	values: FieldValues,
	newest: Readonly<Record<string, CopyValue>> | null,
) => boolean;

/** Desktop cursor after a run started from the rail; null when nothing is per run (it stays). */
export type FocusAfterRun = (
	fields: readonly WorkbenchField[],
	values: FieldValues,
	perRunNames: readonly string[],
	skip: readonly string[],
) => FieldKey | null;

export type ShortcutGroupId = "run" | "move" | "faster" | "dates";
export type ShortcutActionId =
	| "run"
	| "runLeave"
	| "stop"
	| "enterNext"
	| "tabNext"
	| "filter"
	| "runTabs"
	| "recent"
	| "acceptSuggestion"
	| "resetField"
	| "chooseFiles"
	| "replaceFile"
	| "removeFile"
	| "undo"
	| "calendar"
	| "nudge"
	| "presets"
	| "savePreset"
	| "dateDay"
	| "dateDayMonth"
	| "dateFull"
	| "dateWords";
export interface ShortcutRow {
	readonly action: ShortcutActionId;
	readonly keys: readonly string[];
	/** Date rows: the ISO date the example reads as. */
	readonly example?: string;
}
export interface ShortcutGroup {
	readonly id: ShortcutGroupId;
	readonly rows: readonly ShortcutRow[];
}

// ─── Per run (M1), next files (M3), questions ──────────────────────────────

export type AfterRunBack =
	| { readonly kind: "nextFile" }
	| { readonly kind: "empty" }
	| { readonly kind: "on" }
	| { readonly kind: "off" }
	| { readonly kind: "objectDefault" }
	| { readonly kind: "value"; readonly text: string };

/** A row of the "Per run" popover; `back` is the starting value under the active preset. */
export interface AfterRunRow {
	readonly name: string;
	readonly label: string;
	readonly on: boolean;
	readonly back: AfterRunBack;
}

/** The dock message when a run starts from the rail (`flpStartMessage`); the UI joins `pairs`. */
export interface StartMessage {
	readonly n: number;
	/** The run's state now: the reducer follows the run until the message goes. */
	readonly state: "started" | "queued" | "sending";
	readonly files: number;
	/**
	 * What the run took in its per-run fields, in field order, empty ones left out, secrets as SECRET_MASK. Words the
	 * reducer cannot translate are English here; the dock re-pairs from the run's copy (`runId`) with the viewer's words.
	 */
	readonly pairs: readonly string[];
	/** The run the message is about. */
	readonly runId?: string;
	readonly lastFile: boolean;
	/** ⇧⌘↵: "Inputs left as they are this time." */
	readonly leftAsIs: boolean;
}

/** A yes/no question in the dock status line until it is answered. */
export interface DockQuestion {
	/** `offer`: "Make A and B per run?" (once per form). `seriesEnd`: "Should A, B and C stay per run?" (`prefs.auto`). */
	readonly kind: "offer" | "seriesEnd";
	readonly names: readonly string[];
}

/** The open list under a field: recent values (M6) or next files (M3). One at a time. */
export interface OpenList {
	readonly kind: "recent" | "nextFiles";
	readonly key: FieldKey;
	/** Active row; −1 for none. */
	readonly active: number;
}

// ─── Runs ───────────────────────────────────────────────────────────────────

/** The rail as a whole, derived: problems after a press, files of the current entry sending, or neither. */
export type FormPhase = "idle" | "invalid" | "uploading";

/**
 * sending → its own files upload, holds no place · queued → waits for a place (cap of its target) ·
 * starting → dispatched, no sign of the run yet, no clock (also while the execution service shows a
 * pre-run dialog, which the form cannot see) · asking → the flow asks the person in the run
 * (interaction_request): holds its place, never holds the queue · running → started, no answer
 * text · streaming → answer text arriving · done / empty (nothing returned) / failed / stopped /
 * notStarted → ended · unknown → read back from history after the page closed mid-run.
 */
export type RunStatus =
	| "sending"
	| "queued"
	| "starting"
	| "asking"
	| "running"
	| "streaming"
	| "done"
	| "empty"
	| "failed"
	| "stopped"
	| "notStarted"
	| "unknown";

export const LIVE_RUN_STATUSES: readonly RunStatus[] = [
	"sending",
	"queued",
	"starting",
	"asking",
	"running",
	"streaming",
];
/** Statuses that hold one of the parallel places (`flpStartable`). */
export const BUSY_RUN_STATUSES: readonly RunStatus[] = [
	"starting",
	"asking",
	"running",
	"streaming",
];

export type FailureKind =
	| "flow"
	| "upload"
	| "permission"
	| "quota"
	| "oauth"
	| "network"
	| "timeout"
	| "rejected"
	| "other";

export type NotStartedReason =
	| "removedFromQueue"
	| "formClosed"
	| "fileNotSent"
	| "declined"
	| "promptCancelled";

/**
 * `noPlace`: the API refused the start with `concurrent_cloud_executions`; not a failure, the run
 * goes back to the front of the queue as "waiting for a free place" (M5).
 */
export type RunOutcome =
	| { readonly kind: "succeeded" }
	| {
			readonly kind: "failed";
			readonly failure: FailureKind;
			/** Plain message safe to show (serverMessage, unwrapped native error, stream error text). */
			readonly message: string | null;
			/** The raw line for "Details" (error text, log level, run id). */
			readonly detail: string | null;
	  }
	| { readonly kind: "stopped" }
	| { readonly kind: "notStarted"; readonly reason: NotStartedReason }
	| { readonly kind: "noPlace" }
	| { readonly kind: "unknown" };

/** How the engine promise ended. */
export type RunSettlement =
	| { readonly kind: "resolved"; readonly meta: ILogMetadata | null }
	| { readonly kind: "rejected"; readonly error: unknown };

export interface TerminalSignals {
	readonly runInitiated: boolean;
	/** The error event's text; `""` for an error event without text, null when no error event came. */
	readonly errorMessage: string | null;
	/** `completed.payload.status`, normalised (success/succeeded → completed, canceled → cancelled, timed_out → timeout, anything else → failed). Authoritative on every transport. */
	readonly completedStatus:
		| "completed"
		| "failed"
		| "cancelled"
		| "timeout"
		| null;
	/** `<stage>` of `completed.payload.current_step` = `rejected:<stage>`. */
	readonly rejectedStage: string | null;
	readonly logLevel: number | null;
	readonly durationMs: number | null;
}

export interface OutcomeInput {
	readonly settlement: RunSettlement;
	readonly terminal: TerminalSignals;
	readonly eventCount: number;
	readonly stopRequested: boolean;
	readonly host: FormHostKind;
}
export type OutcomeOf = (input: OutcomeInput) => RunOutcome;

export type RunStepState = "done" | "active" | "failed" | "stopped" | "planned";
export interface RunStep {
	readonly id: string;
	/** 1-based position: "Step 4". There is no known total. */
	readonly number: number;
	readonly title: string;
	readonly detail: string | null;
	/** current_message of the active step. */
	readonly message: string | null;
	readonly state: RunStepState;
}

export interface NavigateIntent {
	readonly route: string;
	readonly replace: boolean;
	readonly queryParams?: Readonly<Record<string, string>>;
}

/** Everything a run sent back so far. `result` is null when no result event arrived; `{ value: false }` is a result. */
export interface RunOutput {
	readonly eventCount: number;
	readonly steps: readonly RunStep[];
	readonly answer: string;
	readonly reasoning: string | null;
	readonly attachments: readonly IAttachment[];
	readonly result: { readonly value: unknown } | null;
	readonly interactions: readonly IInteractionRequest[];
	readonly terminal: TerminalSignals;
}

/**
 * Stateful reader of one run's intercom events (run/events.ts). Fed once per event from
 * executeEvent's `onLiveEvents`, never from a subscription replay; it lives as long as its run and is
 * never reset, so `takeNavigation` returns each a2ui navigateTo intent exactly once.
 */
export interface RunAccumulator {
	push(events: readonly IIntercomEvent[]): void;
	output(): RunOutput;
	takeNavigation(): readonly NavigateIntent[];
}
export type CreateRunAccumulator = (context: {
	readonly appId: string;
	readonly eventId: string;
}) => RunAccumulator;

export type ResultTone = "good" | "warning" | "critical" | "info";
export interface ResultRow {
	readonly label: string;
	readonly text: string;
	readonly mono: boolean;
	readonly tone: ResultTone | null;
}
export interface ResultGroup {
	readonly title: string;
	readonly rows: readonly ResultRow[];
}
export interface ResultTable {
	readonly title: string;
	readonly head: readonly string[];
	readonly numeric: readonly boolean[];
	readonly rows: readonly (readonly string[])[];
}
/** A returned value prepared for reading: never a raw JSON dump; the raw text sits behind a toggle. */
export type ResultView =
	| { readonly kind: "text"; readonly text: string; readonly markdown: boolean }
	| {
			readonly kind: "value";
			readonly text: string;
			readonly mono: boolean;
			readonly tone: ResultTone | null;
	  }
	| {
			readonly kind: "structured";
			readonly groups: readonly ResultGroup[];
			readonly tables: readonly ResultTable[];
	  };
export interface ResultModel {
	readonly view: ResultView;
	readonly raw: string;
	readonly copyText: string;
}
export type ToResultModel = (value: unknown) => ResultModel;

export interface StepRef {
	readonly number: number;
	readonly title: string;
}

/** What lists and history show without the full output. */
export interface RunSummary {
	readonly firstLine: string | null;
	readonly stepCount: number;
	readonly stepReached: StepRef | null;
	readonly fileCount: number;
	readonly hasAnswer: boolean;
	readonly hasResult: boolean;
}

/** A run's own copy of the inputs, taken when it was pressed. Nothing typed later reaches it. */
export interface RunCopy {
	/** Files as their slots, also while still sending; secrets keep their real value in memory only. */
	readonly values: Readonly<Record<string, CopyValue>>;
	readonly presetName: string | null;
	/** ⇧⌘↵: per-run fields were left as they were. */
	readonly leftAsIs: boolean;
	/** Fields that were per run when this run took its copy. */
	readonly perRun: readonly string[];
	/** Text fields whose entry began in an empty field or over a fully selected value (the offer). */
	readonly replaced: readonly string[];
}

export interface RunEntry {
	/** Stable unique key (also the stored record's id). */
	readonly id: string;
	/** Display number; two windows may show the same number, rows never collide. */
	readonly n: number;
	readonly origin: "session" | "history";
	readonly status: RunStatus;
	readonly createdAt: number;
	/** First sign of the run (run id or first event); the clock runs from here. */
	readonly startedAt: number | null;
	readonly endedAt: number | null;
	readonly copy: RunCopy;
	readonly target: ExecutionTarget | null;
	readonly streamId: string | null;
	readonly backendRunId: string | null;
	/** Session runs only. */
	readonly output: RunOutput | null;
	readonly outcome: RunOutcome | null;
	readonly summary: RunSummary;
	readonly stopRequested: boolean;
	/** File slot ids this run waits for while `sending`. */
	readonly pendingSlotIds: readonly string[];
	/** Queued again after a `noPlace` refusal: "Queued · waiting for a free place". */
	readonly waitingForPlace: boolean;
	/** The step a failed run ended at (failure line, hold, live region). */
	readonly failedAt: StepRef | null;
	/** A failure the person has not picked yet (tab, Runs row, menu or Show). */
	readonly unseenFailure: boolean;
}

export interface QueueSummary {
	readonly busy: number;
	readonly queued: number;
	readonly sending: number;
	readonly held: boolean;
}

/** Two failures in a row at the same step (`flpHold`): nothing queued starts until "Resume". */
export interface HoldInfo {
	readonly runs: readonly number[];
	readonly step: StepRef;
}

export interface QueueState {
	/** The app host's resolved target; null until resolved (cap: unknown tier). */
	readonly target: ExecutionTarget | null;
	/** The tier's max_concurrent_executions; null while unknown; −1 for no limit. */
	readonly tierLimit: number | null;
	/** `flpCap(target, tierLimit)`; −1 for no cap. */
	readonly cap: number;
	readonly hold: HoldInfo | null;
	/** A refused start is tried again at this time (ms epoch), or earlier when a run of this window ends. */
	readonly retryAt: number | null;
}

// ─── Device memory (§5) ─────────────────────────────────────────────────────

export type JsonValue =
	| string
	| number
	| boolean
	| null
	| readonly JsonValue[]
	| { readonly [key: string]: JsonValue };

export interface StoredFileMark {
	readonly $file: { readonly name: string; readonly size: number | null };
}
export interface StoredHiddenMark {
	readonly $hidden: true;
}
/** A rail value as stored: JSON, files as names and sizes, secrets as a mark. */
export type StoredInput =
	| JsonValue
	| StoredFileMark
	| readonly StoredFileMark[]
	| StoredHiddenMark;

/** `queued` and `sending` read back as notStarted (formClosed), `starting`/`running` as unknown. */
export type StoredRunStatus =
	| "queued"
	| "sending"
	| "running"
	| "done"
	| "empty"
	| "failed"
	| "stopped"
	| "notStarted";

export interface StoredRunRecord {
	readonly version: 2;
	/** Unique row key, the RunEntry id. */
	readonly id: string;
	readonly scope: string;
	readonly appId: string;
	readonly eventId: string;
	readonly n: number;
	readonly createdAt: number;
	readonly startedAt: number | null;
	readonly endedAt: number | null;
	readonly status: StoredRunStatus;
	readonly outcome: RunOutcome | null;
	readonly summary: RunSummary;
	readonly failedAt: StepRef | null;
	readonly presetName: string | null;
	readonly inputs: Readonly<Record<string, StoredInput>>;
	readonly perRun: readonly string[];
	readonly replaced: readonly string[];
}

export interface FormPrefs {
	readonly version: 2;
	/** Fields that are per run (M1). */
	readonly perRun: readonly string[];
	/** Per run only for the current series, made by a multi-file pick; asked about when it ends. */
	readonly auto: readonly string[];
	/** The setting was shown once on this device: the after-run line exists from now on. */
	readonly introduced: boolean;
	readonly offerAnswered: boolean;
	/** "Don't save {label} on this device". */
	readonly noSave: readonly string[];
	/** Recent values deleted from the list, per field, as `hashValue` of the folded text. */
	readonly forgotten: Readonly<Record<string, readonly string[]>>;
	/** Monotonic per window; the store keeps the larger of stored and written. */
	readonly nextRunNumber: number;
	/** Last time each field name existed in the form; names unseen for FORM_LIMITS.prefsPruneMs are pruned. */
	readonly fieldSeenAt: Readonly<Record<string, number>>;
}

export interface Preset {
	readonly id: string;
	readonly name: string;
	/** 1–9 given at creation by the lowest free digit (`flpNextDigit`), kept until deleted; 0 = none. */
	readonly digit: number;
	/** No files, no secrets. */
	readonly sets: Readonly<Record<string, StoredInput>>;
	readonly kinds: Readonly<Record<string, FieldKind>>;
	/** "On open": applies silently when the form opens. At most one per form. */
	readonly openDefault: boolean;
	readonly createdAt: number;
	readonly updatedAt: number;
	readonly lastUsedAt: number | null;
}

export interface PresetDraft {
	readonly name: string;
	readonly openDefault: boolean;
	readonly ticked: readonly string[];
	/** "Save these inputs as a preset…" from a run's menu uses that run's copy. */
	readonly fromRunId: string | null;
	/** Saving under an existing name replaces that preset. */
	readonly replaceId: string | null;
}

export interface FormMemoryKey {
	/** `HostCapabilities.memoryScope`, or a fixed session key with session persistence. */
	readonly scope: string;
	readonly appId: string;
	readonly eventId: string;
}

export interface LoadedMemory {
	readonly prefs: FormPrefs | null;
	readonly presets: readonly Preset[];
	/** Newest first, at most FORM_LIMITS.historyPerForm. */
	readonly runs: readonly StoredRunRecord[];
}

/** Every method swallows storage errors (private windows, blocked storage): the form works from memory. */
export interface FormMemoryStore {
	readonly mode: FormPersistence;
	load(key: FormMemoryKey): Promise<LoadedMemory>;
	putRun(record: StoredRunRecord): Promise<void>;
	deleteRun(key: FormMemoryKey, id: string): Promise<void>;
	/** "Don't save {label}": rewrites that input to `{ $hidden: true }` in this form's saved runs. */
	hideField(key: FormMemoryKey, name: string): Promise<void>;
	putPrefs(key: FormMemoryKey, prefs: FormPrefs): Promise<void>;
	putPreset(key: FormMemoryKey, preset: Preset): Promise<void>;
	deletePreset(key: FormMemoryKey, presetId: string): Promise<void>;
}
/** `scope`: a per-page-session map (runtimeMemory(appId) for a Devices runtime namespace), else a module map. */
export type CreateFormMemoryStore = (
	mode: FormPersistence,
	scope: Map<string, unknown> | null,
) => FormMemoryStore;

export interface RecentValue {
	readonly value: string;
	/** ms epoch of the run that had it; the UI prints "Today 14:02", "Yesterday", "Mon 28 Sep". */
	readonly at: number;
}
export interface RecentSourceEntry {
	readonly at: number;
	readonly values: Readonly<Record<string, CopyValue | StoredInput>>;
}
export interface FieldRecall {
	readonly source: readonly RecentSourceEntry[];
	/** Hashes (`hashValue`) of values deleted from the list. */
	readonly forgotten: readonly string[];
}

// ─── Dock ───────────────────────────────────────────────────────────────────

export type DockMessage =
	| { readonly kind: "start"; readonly start: StartMessage }
	| {
			readonly kind: "sameInputs";
			readonly n: number;
	  }
	| { readonly kind: "perRunNow"; readonly labels: readonly string[] }
	| { readonly kind: "fieldReset"; readonly label: string }
	| { readonly kind: "fileRemoved"; readonly name: string }
	| { readonly kind: "leftOut"; readonly files: readonly LeftOutFile[] }
	| {
			readonly kind: "presetApplied";
			readonly name: string;
			readonly misfit: number;
	  }
	| { readonly kind: "presetUpdated"; readonly name: string }
	| { readonly kind: "presetSaved"; readonly name: string }
	| { readonly kind: "presetDeleted"; readonly name: string }
	| { readonly kind: "resetTo"; readonly presetName: string | null }
	| {
			readonly kind: "inputsIn";
			readonly n: number;
			readonly pickAgain: number;
			readonly enterAgain: readonly string[];
			readonly misfit: number;
	  }
	| { readonly kind: "noSave"; readonly label: string }
	| { readonly kind: "nextFilesRemoved"; readonly count: number }
	| { readonly kind: "nextFilesCapped" }
	| {
			readonly kind: "queueCleared";
			readonly runs: number;
			readonly files: number;
	  }
	| {
			readonly kind: "fileRefused";
			readonly name: string;
			readonly limitBytes: number;
			readonly host: FormHostKind;
	  }
	| { readonly kind: "fileWarning"; readonly name: string }
	| { readonly kind: "oneFileOnly"; readonly name: string }
	| {
			readonly kind: "requestTooLarge";
			readonly totalBytes: number;
			readonly limitBytes: number;
	  }
	/** An answer to a run's question did not reach the run; the question stays open. Goes when the run ends. */
	| {
			readonly kind: "answerFailed";
			readonly n: number;
			readonly runId: string;
	  };

/** `expiresAt`: start and plain messages after FORM_LIMITS.messageMs; a message with Undo stays until the next edit or run. */
export interface DockMessageEntry {
	readonly seq: number;
	readonly message: DockMessage;
	readonly undo: boolean;
	readonly expiresAt: number | null;
}

export interface FailureLine {
	readonly runId: string;
	readonly n: number;
	readonly step: StepRef | null;
}

/** The dock's single status line, highest first (`flpDockLine`). */
export type DockLine =
	| {
			readonly kind: "sending";
			readonly current: number;
			readonly total: number;
	  }
	| { readonly kind: "problems"; readonly count: number }
	| { readonly kind: "hold"; readonly hold: HoldInfo }
	| { readonly kind: "message"; readonly entry: DockMessageEntry }
	| { readonly kind: "failure"; readonly runs: readonly FailureLine[] }
	| { readonly kind: "question"; readonly question: DockQuestion }
	| {
			readonly kind: "queue";
			readonly running: number;
			readonly queued: number;
	  }
	| { readonly kind: "blocked"; readonly label: string }
	| { readonly kind: "compared"; readonly n: number; readonly changes: number }
	| { readonly kind: "missing"; readonly count: number }
	| { readonly kind: "ready" };

export interface DockLineInput {
	readonly sending: { readonly left: number; readonly total: number } | null;
	readonly problems: number;
	readonly hold: HoldInfo | null;
	readonly message: DockMessageEntry | null;
	readonly failures: readonly FailureLine[];
	readonly question: DockQuestion | null;
	readonly running: number;
	readonly queued: number;
	/** Labels of required FlowPath fields this host cannot fill. */
	readonly blocked: readonly string[];
	readonly compared: { readonly n: number; readonly changes: number } | null;
	readonly missing: number;
}
export type DockLineOf = (input: DockLineInput) => DockLine;

// ─── Session state ──────────────────────────────────────────────────────────

export interface FormModel {
	readonly appId: string;
	readonly eventId: string;
	readonly nodeId: string;
	readonly name: string;
	readonly description: string;
	readonly fields: readonly WorkbenchField[];
	/** Content key of fields, routes, labels and texts: the hook's change detection only, never stored. */
	readonly contentKey: string;
	/** `config.submit_label` when it is a non-empty string. */
	readonly submitLabel: string | null;
	/** `config.navigate_to_routes`, normalised and deduplicated. */
	readonly routes: readonly string[];
	readonly eventRoute: string | null;
	readonly host: HostCapabilities;
	readonly viewer: ViewerHabits;
}

/** The form model of one event (`model/fields.ts` `createFormModel`). `appId` comes from the host: `IEvent` carries none. */
export type CreateFormModel = (
	event: IEvent,
	config: Partial<IEventPayload> | undefined,
	host: HostCapabilities,
	viewer: ViewerHabits,
	today: string,
	appId: string,
) => FormModel;

export type RailTab = "inputs" | "runs";
export type Pane = "inputs" | "output";
export interface RailFilter {
	readonly query: string;
	readonly chip: "all" | "required" | "changed";
}

export interface RailState {
	/** By field name; a group's properties live inside its GroupValue. */
	readonly values: FieldValues;
	/** Uncommitted text of date and number controls, by FieldKey. */
	readonly texts: Readonly<Record<FieldKey, string>>;
	readonly problems: Readonly<Record<FieldKey, FieldProblem>>;
	/** Run was pressed while something needed a look; the dock shows the problem line. */
	readonly pressed: boolean;
	/** Files waiting in a one-file field (M3), by field name. Never stored. */
	readonly nextFiles: Readonly<Record<string, readonly FileSlot[]>>;
	/** Picked files left out because a run on this device already had them, by field name ("Add it"). */
	readonly leftOut: Readonly<Record<string, readonly LeftOutFile[]>>;
	/** Last date entered per field in this session; the anchor falls back to the newest run, then today (`dateAnchorOf`). */
	readonly dateAnchors: Readonly<Record<FieldKey, string>>;
	/** How each text field's entry began since the last run: the first change decides (the offer's `replaced`). */
	readonly entryBegan: Readonly<Record<string, "replaced" | "inPlace">>;
	readonly activePresetId: string | null;
	/** The run the rail is compared with; null when the stage shows only context. */
	readonly comparedRunId: string | null;
	readonly tab: RailTab;
	readonly filter: RailFilter;
	readonly lastPressAt: number | null;
}

export type Overlay =
	| { readonly id: "afterRun"; readonly focusName: string | null }
	| { readonly id: "shortcuts" }
	| { readonly id: "presets" }
	| {
			readonly id: "presetSave";
			readonly mode: "save" | "update";
			readonly fromRunId: string | null;
	  }
	| { readonly id: "runMenu"; readonly runId: string }
	| { readonly id: "moreRuns" }
	| {
			readonly id: "leave";
			readonly route: string;
			readonly replace: boolean;
	  };

export type FocusTarget =
	| {
			readonly kind: "field";
			readonly key: FieldKey;
			/** Select the whole value on arrival; otherwise the caret goes after it. */
			readonly select: boolean;
			/**
			 * After a run: a box without the split (a phone) scrolls it into view without focus, so no
			 * keyboard opens; a split box gives it focus as usual.
			 */
			readonly scrollOnly: boolean;
	  }
	| { readonly kind: "run" }
	| { readonly kind: "runAgain" }
	| { readonly kind: "stop" }
	| { readonly kind: "change" }
	| { readonly kind: "copy" }
	| { readonly kind: "presetButton" }
	| { readonly kind: "filter" }
	| { readonly kind: "tab"; readonly runId: string };

export interface FocusRequest {
	readonly seq: number;
	readonly target: FocusTarget;
}

export interface ViewState {
	readonly selectedRunId: string | null;
	readonly pinnedRunId: string | null;
	readonly pane: Pane;
	readonly overlay: Overlay | null;
	readonly focus: FocusRequest | null;
	readonly message: DockMessageEntry | null;
	/** The offer or the series-end question; stays until answered. */
	readonly question: DockQuestion | null;
	readonly list: OpenList | null;
	/** Phone: the Output segment's dot. */
	readonly outputUnseen: boolean;
	/** A starting run takes the stage only while the stage shows the newest started run. */
	readonly stageFollowsNewest: boolean;
}

export interface MemoryState {
	readonly loaded: boolean;
	readonly prefs: FormPrefs;
	readonly presets: readonly Preset[];
}

/** One level, in memory; cleared by the next edit or run (no timer). Restores what the reported change touched. */
export interface UndoEntry {
	readonly seq: number;
	readonly kind:
		| "preset"
		| "update"
		| "resetAll"
		| "resetTo"
		| "fieldReset"
		| "useInputs"
		| "deletePreset"
		| "fileRemoved"
		| "nextFilesRemoved"
		| "queueCleared";
	readonly values: FieldValues;
	readonly nextFiles: Readonly<Record<string, readonly FileSlot[]>>;
	readonly activePresetId: string | null;
	readonly deletedPreset: Preset | null;
}

/** Polite live region in the stage: run ends and a refused zero-field press; never the ticking clock. */
export type Announcement =
	| {
			readonly seq: number;
			readonly kind: "done" | "empty" | "failed" | "stopped" | "unknown";
			readonly n: number;
			readonly seconds: number;
			readonly step: StepRef | null;
	  }
	| {
			readonly seq: number;
			readonly kind: "capReached";
			readonly running: number;
	  };

/** The single source the UI renders from. Immutable; one new object per change. */
export interface FormSessionState {
	readonly form: FormModel;
	readonly layout: WorkbenchLayout | null;
	readonly rail: RailState;
	/** Newest first: this session's runs, then history read from memory. */
	readonly runs: readonly RunEntry[];
	readonly view: ViewState;
	readonly memory: MemoryState;
	readonly queue: QueueState;
	readonly announcement: Announcement | null;
	readonly undo: UndoEntry | null;
	readonly seq: number;
}

// ─── Session messages ───────────────────────────────────────────────────────

export interface SessionClock {
	readonly now: number;
	/** Local calendar day, `YYYY-MM-DD`. */
	readonly today: string;
}

export type RunTrigger =
	| "button"
	| "chord"
	| "enter"
	| "hero"
	| "strip"
	| "phone";
export type SelectHow = "tab" | "list" | "menu" | "show" | "keyboard" | "auto";
/** How a text change began: over an empty field or a fully selected value, or inside the value. */
export type EditHow = "replace" | "inPlace";

export type SessionCommand =
	| {
			readonly type: "setValue";
			readonly key: FieldKey;
			readonly value: FieldValue;
			readonly how?: EditHow;
	  }
	| { readonly type: "setText"; readonly key: FieldKey; readonly text: string }
	| { readonly type: "commitText"; readonly key: FieldKey }
	| { readonly type: "blurField"; readonly key: FieldKey }
	| { readonly type: "resetField"; readonly key: FieldKey }
	| { readonly type: "resetAll" }
	| {
			readonly type: "pickFiles";
			readonly name: string;
			readonly files: readonly PickedFile[];
			readonly mode: "replace" | "append";
	  }
	| {
			readonly type: "removeFile";
			readonly name: string;
			readonly slotId: string;
	  }
	| { readonly type: "retryFile"; readonly slotId: string }
	| {
			readonly type: "removeNextFile";
			readonly name: string;
			readonly slotId: string;
	  }
	| { readonly type: "clearNextFiles"; readonly name: string }
	| { readonly type: "addLeftOut"; readonly name: string }
	| {
			readonly type: "run";
			readonly leaveAsIs: boolean;
			readonly from: RunTrigger;
	  }
	| { readonly type: "enter"; readonly fromKey: FieldKey }
	| { readonly type: "runAgain"; readonly runId: string }
	| { readonly type: "stop"; readonly runId: string }
	| { readonly type: "removeFromQueue"; readonly runId: string }
	| { readonly type: "clearQueue" }
	| { readonly type: "resumeQueue" }
	| { readonly type: "useInputs"; readonly runId: string }
	| { readonly type: "removeRun"; readonly runId: string }
	| {
			readonly type: "selectRun";
			readonly runId: string | null;
			readonly how: SelectHow;
	  }
	| { readonly type: "pinRun"; readonly runId: string | null }
	| { readonly type: "setRailTab"; readonly tab: RailTab }
	| { readonly type: "setPane"; readonly pane: Pane }
	| { readonly type: "setFilter"; readonly filter: Partial<RailFilter> }
	| { readonly type: "openOverlay"; readonly overlay: Overlay }
	| { readonly type: "closeOverlay" }
	| {
			readonly type: "openList";
			readonly kind: OpenList["kind"];
			readonly key: FieldKey;
	  }
	| { readonly type: "moveList"; readonly delta: number }
	| { readonly type: "closeList" }
	| {
			readonly type: "setPerRun";
			readonly name: string;
			readonly on: boolean;
	  }
	| { readonly type: "uncheckAllPerRun" }
	| { readonly type: "perRunFilesAndDates" }
	| { readonly type: "answerQuestion"; readonly yes: boolean }
	| {
			readonly type: "forgetRecent";
			readonly name: string;
			readonly value: string;
	  }
	| { readonly type: "dontSave"; readonly name: string }
	| { readonly type: "applyPreset"; readonly presetId: string | null }
	| { readonly type: "savePreset"; readonly draft: PresetDraft }
	| { readonly type: "updatePreset"; readonly presetId: string }
	| { readonly type: "deletePreset"; readonly presetId: string }
	| { readonly type: "resetToPreset" }
	| { readonly type: "undo" }
	| {
			readonly type: "respondInteraction";
			readonly runId: string;
			readonly interactionId: string;
			readonly value: unknown;
	  }
	| { readonly type: "dismissMessage" }
	/** A key press in the rail: ends a start message early. */
	| { readonly type: "railKey" }
	| { readonly type: "focusHandled"; readonly seq: number }
	| { readonly type: "markSeen"; readonly runId: string }
	| { readonly type: "setLayout"; readonly layout: WorkbenchLayout };

/** What the runtime reports back. */
export type SessionInput =
	| { readonly type: "memoryLoaded"; readonly memory: LoadedMemory }
	| {
			readonly type: "targetResolved";
			readonly target: ExecutionTarget;
			readonly tierLimit: number | null;
	  }
	| {
			readonly type: "uploadProgress";
			readonly slotId: string;
			readonly progress: number;
	  }
	| {
			readonly type: "uploadSent";
			readonly slotId: string;
			readonly ref: FileRef;
			readonly expiresAt: number | null;
	  }
	| {
			readonly type: "uploadFailed";
			readonly slotId: string;
			readonly error: string;
	  }
	| {
			readonly type: "runAccepted";
			readonly runId: string;
			readonly backendRunId: string;
	  }
	| {
			readonly type: "runOutput";
			readonly runId: string;
			readonly output: RunOutput;
	  }
	| {
			readonly type: "runSettled";
			readonly runId: string;
			readonly outcome: RunOutcome;
	  }
	/** `respondInteraction` could not send the answer (the run ended, the question is unknown or the channel failed). */
	| {
			readonly type: "interactionFailed";
			readonly runId: string;
			readonly interactionId: string;
			readonly error: string;
	  }
	| { readonly type: "messageExpired"; readonly seq: number }
	| { readonly type: "retryDue" }
	/** The host passed an event or config with a different `contentKey`: values are rebased by name, runs stay. */
	| { readonly type: "formChanged"; readonly form: FormModel }
	| { readonly type: "detached" }
	| { readonly type: "attached" };

export type SessionMessage =
	| { readonly kind: "command"; readonly command: SessionCommand }
	| { readonly kind: "input"; readonly input: SessionInput };

/**
 * What the reducer asks the runtime to do. Slots are reference-counted from state (rail values,
 * next files, left-out files, every run copy): `abortUpload` and `releaseFiles` are emitted only
 * when the last holder of a slot drops it.
 */
export type SessionEffect =
	| {
			readonly type: "upload";
			readonly slotId: string;
			readonly name: string;
			readonly mode: FileMode;
	  }
	| { readonly type: "abortUpload"; readonly slotId: string }
	| { readonly type: "releaseFiles"; readonly slotIds: readonly string[] }
	| { readonly type: "dispatchRun"; readonly runId: string }
	| { readonly type: "stopRun"; readonly runId: string }
	| { readonly type: "persistRun"; readonly record: StoredRunRecord }
	| { readonly type: "deleteRun"; readonly id: string }
	| { readonly type: "hideField"; readonly name: string }
	| { readonly type: "persistPrefs"; readonly prefs: FormPrefs }
	| { readonly type: "persistPreset"; readonly preset: Preset }
	| { readonly type: "deletePreset"; readonly presetId: string }
	| {
			readonly type: "expireMessage";
			readonly seq: number;
			readonly afterMs: number;
	  }
	| { readonly type: "scheduleRetry"; readonly afterMs: number }
	| {
			readonly type: "respondInteraction";
			readonly runId: string;
			readonly interactionId: string;
			readonly value: unknown;
	  };

export interface SessionStep {
	readonly state: FormSessionState;
	readonly effects: readonly SessionEffect[];
}

export type ReduceSession = (
	state: FormSessionState,
	message: SessionMessage,
	clock: SessionClock,
) => SessionStep;

export type InitialSessionState = (
	form: FormModel,
	clock: SessionClock,
) => FormSessionState;

/** UI-facing actions. The hook adds the clock, keeps File objects and dispatches commands. */
export interface FormSessionActions {
	setValue(key: FieldKey, value: FieldValue, how?: EditHow): void;
	setText(key: FieldKey, text: string): void;
	commitText(key: FieldKey): void;
	blurField(key: FieldKey): void;
	resetField(key: FieldKey): void;
	resetAll(): void;
	pickFiles(
		name: string,
		files: readonly File[],
		mode: "replace" | "append",
	): void;
	removeFile(name: string, slotId: string): void;
	retryFile(slotId: string): void;
	removeNextFile(name: string, slotId: string): void;
	clearNextFiles(name: string): void;
	addLeftOut(name: string): void;
	run(options?: {
		readonly leaveAsIs?: boolean;
		readonly from?: RunTrigger;
	}): void;
	enter(fromKey: FieldKey): void;
	runAgain(runId: string): void;
	stop(runId: string): void;
	removeFromQueue(runId: string): void;
	clearQueue(): void;
	resumeQueue(): void;
	useInputs(runId: string): void;
	removeRun(runId: string): void;
	selectRun(runId: string | null, how: SelectHow): void;
	pinRun(runId: string | null): void;
	setRailTab(tab: RailTab): void;
	setPane(pane: Pane): void;
	setFilter(filter: Partial<RailFilter>): void;
	openOverlay(overlay: Overlay): void;
	closeOverlay(): void;
	openList(kind: OpenList["kind"], key: FieldKey): void;
	moveList(delta: number): void;
	closeList(): void;
	setPerRun(name: string, on: boolean): void;
	uncheckAllPerRun(): void;
	perRunFilesAndDates(): void;
	answerQuestion(yes: boolean): void;
	forgetRecent(name: string, value: string): void;
	dontSave(name: string): void;
	applyPreset(presetId: string | null): void;
	savePreset(draft: PresetDraft): void;
	updatePreset(presetId: string): void;
	deletePreset(presetId: string): void;
	resetToPreset(): void;
	undo(): void;
	respondInteraction(
		runId: string,
		interactionId: string,
		value: unknown,
	): void;
	dismissMessage(): void;
	railKey(): void;
	focusHandled(seq: number): void;
	markSeen(runId: string): void;
	setLayout(layout: WorkbenchLayout): void;
}

export interface UseFormSessionInput {
	readonly appId: string;
	readonly event: IEvent;
	readonly config: Partial<IEventPayload> | undefined;
	readonly host: HostCapabilities;
	readonly viewer: ViewerHabits;
	/** a2ui navigateTo from a run; the shell's navigate (onNavigate, else the client router). */
	readonly navigate: (intent: NavigateIntent) => void;
}

export interface FormSessionHandle {
	readonly state: FormSessionState;
	readonly actions: FormSessionActions;
}

export type UseFormSession = (input: UseFormSessionInput) => FormSessionHandle;

// ─── Component props ────────────────────────────────────────────────────────

/** The form's own route buttons (header pills, "Go to …" after a run); `go` asks first while runs or next files wait. */
export interface RouteNav {
	/** Route → label: the routed event's name, "Home" for "/", else the prettified path. */
	readonly labels: Readonly<Record<string, string>>;
	go(route: string): void;
}

export interface WorkbenchViewProps {
	readonly state: FormSessionState;
	readonly actions: FormSessionActions;
	readonly layout: WorkbenchLayout;
	readonly routes: RouteNav;
}

/** Rail column above the dock: head, presets, Inputs/Runs, filter, field list or Runs list. */
export type RailProps = WorkbenchViewProps;

/**
 * The one Run control family. `rail`: bottom of the desktop rail (status line, Run, Stop, after-run
 * line). `phone`: the narrow layout's bottom bar. `hero`: Run inside the zero-field card before the
 * first run. `strip`: the coral Run / Run again at the left end of a zero-field form's run strip.
 */
export interface DockProps extends WorkbenchViewProps {
	readonly variant: "rail" | "phone" | "hero" | "strip";
}

/** Run strip, run bar, sections, compare, empty stage. */
export type StageProps = WorkbenchViewProps;

export interface FieldMarkers {
	/** 6 px dot (`flpChanged`): differs from its starting value, only for fields with a default or set by the active preset, never per run. */
	readonly changed: boolean;
	/** 2 px inset edge: differs from the compared run; per-run fields never. */
	readonly differs: boolean;
	readonly perRun: "perRun" | "nextFile" | null;
	readonly optional: boolean;
	/** Hover/focus text button (tabIndex −1): "Reset" to the starting value, "Clear" when there is none. */
	readonly reset: "reset" | "clear" | null;
	/** "2 files". */
	readonly count: number | null;
}

/**
 * One control with its label line, help and message. Groups render their properties with nested
 * FieldControls and pass the same callbacks, so every property gets its own markers, recent values
 * and date anchor (the reducer commits typed dates against the same `dateAnchorFor`).
 */
export interface FieldControlProps {
	readonly field: WorkbenchField;
	readonly rail: RailState;
	readonly markersFor: (key: FieldKey) => FieldMarkers;
	/** Null for fields without recent values (non-text kinds, secrets, "Don't save", coarse pointer). */
	readonly recallFor: (key: FieldKey) => FieldRecall | null;
	readonly dateAnchorFor: (key: FieldKey) => string;
	readonly enterHint: (key: FieldKey) => "next" | "go";
	/** A FlowPath field this host cannot fill: the disabled row, no Tab stop. */
	readonly blocked: boolean;
	readonly list: OpenList | null;
	readonly host: HostCapabilities;
	readonly viewer: ViewerHabits;
	readonly layout: WorkbenchLayout;
	readonly today: string;
	readonly disabled: boolean;
	readonly actions: FormSessionActions;
}

/** A modal inside the interface box: scrim over the interface only, focus trapped, Esc closes, focus returns. */
export interface InterfaceModalProps {
	readonly open: boolean;
	readonly onClose: () => void;
	readonly labelledBy: string;
	readonly width: number;
	readonly children: ReactNode;
}

export interface WorkbenchShellProps {
	readonly state: FormSessionState;
	readonly actions: FormSessionActions;
	readonly appId: string;
	readonly toolbarRef: RefObject<IToolBarActions | null> | undefined;
	readonly navigate: (intent: NavigateIntent) => void;
}

// ─── Constant tables ────────────────────────────────────────────────────────

/** Tone per status; icons and words are in status-look.ts. Coral is never a status. */
export const RUN_STATUS_TONE: Readonly<Record<RunStatus, Tone>> = {
	sending: "info",
	queued: "unknown",
	starting: "info",
	asking: "info",
	running: "info",
	streaming: "info",
	done: "good",
	empty: "good",
	failed: "critical",
	stopped: "unknown",
	notStarted: "unknown",
	unknown: "unknown",
};
