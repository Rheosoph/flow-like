import {
	DEPLOYMENT_CONFIG_BYTES,
	type DeploymentPlan,
	type DeploymentVariable,
	type PlacementConfiguration,
	validateVariableValue,
	variableText,
	variableValue,
} from "../../../../lib/device-management/deployment";

/* The settings of one service as the device stores them, and what an edit turns them into (SPEC §5.3 Configuration). No React, no I/O. */

export type PlacementConfig = PlacementConfiguration["config"];

export const CONFIG_MAX_BYTES = DEPLOYMENT_CONFIG_BYTES;

const MIB = 1024 ** 2;
const GIB = 1024 ** 3;
const DAY_S = 86_400;

const LOOPBACK = new Set(["127.0.0.1", "::1"]);
const WILDCARD = new Set(["0.0.0.0", "::"]);

export type Exposure = "loopback" | "all" | "one";

export function exposureOf(host: string): Exposure {
	if (LOOPBACK.has(host)) return "loopback";
	return WILDCARD.has(host) ? "all" : "one";
}

export interface HostingView {
	host: string;
	port: number;
	maxInFlight: number;
	timeoutS: number;
	authSecret: string;
	origins: string[];
	exposure: Exposure;
}

const record = (value: unknown): Record<string, unknown> =>
	value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};

const isText = (value: unknown): value is string => typeof value === "string";

export function hostingOf(config: PlacementConfig): HostingView | null {
	const hosting = config.hosting;
	if (!hosting) return null;
	const origins = record(hosting).ui_origins;
	return {
		host: hosting.host,
		port: hosting.port,
		maxInFlight: hosting.max_in_flight,
		timeoutS: hosting.request_timeout_secs,
		authSecret: hosting.auth_secret,
		origins: Array.isArray(origins) ? origins.filter(isText) : [],
		exposure: exposureOf(hosting.host),
	};
}

export interface RestartView {
	initialS: number;
	maxS: number;
	maxRestarts: number;
}

/** `null` when the device sent no restart policy (it then uses its default). */
export function restartOf(config: PlacementConfig): RestartView | null {
	const restart = record(record(config).restart);
	const { initial_backoff_secs, max_backoff_secs, max_restarts } = restart;
	if (
		typeof initial_backoff_secs !== "number" ||
		typeof max_backoff_secs !== "number" ||
		typeof max_restarts !== "number"
	)
		return null;
	return {
		initialS: initial_backoff_secs,
		maxS: max_backoff_secs,
		maxRestarts: max_restarts,
	};
}

const listLength = (value: unknown) =>
	Array.isArray(value) ? value.length : 0;

export function pinCounts(config: PlacementConfig) {
	const raw = record(config);
	return {
		models: listLength(raw.bit_pins),
		packages: listLength(raw.package_pins),
	};
}

/** Bytes of a value as the device receives it: compact JSON in UTF-8. */
export function jsonBytes(value: unknown): number {
	return new TextEncoder().encode(JSON.stringify(value)).length;
}

/** `http(s)://host:port/ui/`; a wildcard address needs the name people use (BG20). */
export function servicePageAddress(
	hosting: Pick<HostingView, "host" | "port" | "exposure">,
	tls: boolean,
	publicHost?: string,
): string | undefined {
	const named = publicHost?.trim();
	const host = hosting.exposure === "all" ? named : hosting.host;
	if (!host) return undefined;
	const authority =
		host.includes(":") && !host.startsWith("[") ? `[${host}]` : host;
	return `${tls ? "https" : "http"}://${authority}:${hosting.port}/ui/`;
}

/* Variable definitions of an online service's flows. */

interface BoardVariableLike {
	id?: unknown;
	name?: unknown;
	data_type?: unknown;
	value_type?: unknown;
	secret?: unknown;
	exposed?: unknown;
	runtime_configured?: unknown;
}

interface BoardLike {
	variables?: Record<string, BoardVariableLike> | null;
	layers?: Record<
		string,
		{ variables?: Record<string, BoardVariableLike> | null }
	> | null;
}

const IDENTIFIER = /^[A-Za-z0-9_.-]{1,128}$/;

/** The variables a flow lets a device set (exposed or set at run time), in the shape the deploy plan uses. */
export function boardDefinitions(board: BoardLike): DeploymentVariable[] {
	const all = [
		...Object.values(board.variables ?? {}),
		...Object.values(board.layers ?? {}).flatMap((layer) =>
			Object.values(layer.variables ?? {}),
		),
	];
	return all
		.filter((variable) => variable.exposed || variable.runtime_configured)
		.flatMap((variable) =>
			typeof variable.id === "string" && IDENTIFIER.test(variable.id)
				? [
						{
							id: variable.id,
							name: String(variable.name ?? "").slice(0, 120),
							data_type: String(variable.data_type),
							value_type: String(variable.value_type),
							secret: variable.secret === true,
						},
					]
				: [],
		);
}

/* Editable fields. */

export type FieldSection =
	| "variables"
	| "instances"
	| "endpoint"
	| "isolation"
	| "buffering";
export type FieldKind =
	| "text"
	| "number"
	| "bool"
	| "json"
	| "select"
	| "secret";
export type FieldUnit = "cores" | "gib" | "mib" | "days" | "seconds";
export type FieldLock =
	| "buffering_single"
	| "background_single"
	| "certificates";
/** A stored value nothing reads any more, or one that no longer fits its type. */
export type VariableIssue = "unused" | "incompatible";

export interface FieldVariable {
	id: string;
	definition?: DeploymentVariable;
	/** The device holds a value for it. */
	stored: boolean;
	issue?: VariableIssue;
}

export interface SettingField {
	id: string;
	section: FieldSection;
	kind: FieldKind;
	/** The current value as text; "" for a variable without a stored value. */
	value: string;
	min?: number;
	max?: number;
	step?: number;
	unit?: FieldUnit;
	options?: readonly string[];
	lock?: FieldLock;
	variable?: FieldVariable;
}

export interface FieldContext {
	/** Variables the served events define; `undefined` while not known. */
	definitions?: readonly DeploymentVariable[];
	/** Certificates on the device, by id. */
	certificates?: readonly string[];
	canAssignCertificate: boolean;
	/** Every served event answers web requests, so more than one instance may run. */
	multiInstance: boolean;
}

const LITERAL_TEXT = new Set(["String", "PathBuf", "Date"]);
const NUMERIC = new Set(["Integer", "Byte", "Float"]);

function definedKind(definition: DeploymentVariable): FieldKind {
	if (definition.value_type !== "Normal") return "json";
	if (LITERAL_TEXT.has(definition.data_type)) return "text";
	if (definition.data_type === "Boolean") return "bool";
	return NUMERIC.has(definition.data_type) ? "number" : "json";
}

function valueKind(value: unknown): FieldKind {
	if (typeof value === "string") return "text";
	if (typeof value === "number") return "number";
	return typeof value === "boolean" ? "bool" : "json";
}

const valueText = (value: unknown) =>
	typeof value === "string" ? value : JSON.stringify(value);

function fits(definition: DeploymentVariable, value: unknown): boolean {
	try {
		validateVariableValue(definition, value);
		return true;
	} catch {
		return false;
	}
}

/** Without definitions nothing is flagged; a stored value nothing defines is unused, one its definition doesn't accept is incompatible. */
function issueOf(
	context: FieldContext,
	definition: DeploymentVariable | undefined,
	accepted: boolean,
): VariableIssue | undefined {
	if (context.definitions === undefined) return undefined;
	if (!definition) return "unused";
	return accepted ? undefined : "incompatible";
}

function storedField(
	id: string,
	shown: Pick<SettingField, "kind" | "value">,
	definition: DeploymentVariable | undefined,
	issue: VariableIssue | undefined,
): SettingField {
	return {
		id: `var.${id}`,
		section: "variables",
		...shown,
		variable: {
			id,
			stored: true,
			...(definition ? { definition } : {}),
			...(issue ? { issue } : {}),
		},
	};
}

function storedVariable(
	id: string,
	value: unknown,
	context: FieldContext,
): SettingField {
	const definition = context.definitions?.find((entry) => entry.id === id);
	const plain = definition && !definition.secret ? definition : undefined;
	const usable = !!plain && fits(plain, value);
	return storedField(
		id,
		{
			kind: plain ? definedKind(plain) : valueKind(value),
			value: usable && plain ? variableText(plain, value) : valueText(value),
		},
		definition,
		issueOf(context, definition, usable),
	);
}

function storedSecret(id: string, context: FieldContext): SettingField {
	const definition = context.definitions?.find((entry) => entry.id === id);
	return storedField(
		id,
		{ kind: "secret", value: "" },
		definition,
		issueOf(context, definition, definition?.secret === true),
	);
}

function variableFields(
	config: PlacementConfig,
	context: FieldContext,
): SettingField[] {
	const stored = [
		...Object.entries(config.variables ?? {}).map(([id, value]) =>
			storedVariable(id, value, context),
		),
		...Object.keys(config.secret_overrides ?? {}).map((id) =>
			storedSecret(id, context),
		),
	];
	const taken = new Set(stored.map((field) => field.variable?.id));
	const open = (context.definitions ?? [])
		.filter((definition) => !definition.secret && !taken.has(definition.id))
		.map(
			(definition): SettingField => ({
				id: `var.${definition.id}`,
				section: "variables",
				kind: definedKind(definition),
				value: "",
				variable: { id: definition.id, definition, stored: false },
			}),
		);
	return [...stored, ...open];
}

const trimmed = (value: number, digits = 3) =>
	String(Number(value.toFixed(digits)));

function endpointFields(
	config: PlacementConfig,
	context: FieldContext,
): SettingField[] {
	const hosting = hostingOf(config);
	if (!hosting) return [];
	const hosts = ["127.0.0.1", "0.0.0.0"];
	const certificate = config.tls_certificate_id ?? "";
	return [
		{
			id: "port",
			section: "endpoint",
			kind: "number",
			value: String(hosting.port),
			min: 1,
			max: 65_535,
			step: 1,
		},
		{
			id: "inflight",
			section: "endpoint",
			kind: "number",
			value: String(hosting.maxInFlight),
			min: 1,
			max: 1024,
			step: 1,
		},
		{
			id: "timeout",
			section: "endpoint",
			kind: "number",
			value: String(hosting.timeoutS),
			min: 1,
			max: 3600,
			step: 1,
			unit: "seconds",
		},
		{
			id: "bind",
			section: "endpoint",
			kind: "select",
			value: hosting.host,
			options: hosts.includes(hosting.host) ? hosts : [...hosts, hosting.host],
		},
		{
			id: "cert",
			section: "endpoint",
			kind: "select",
			value: certificate,
			options: [
				"",
				...new Set([
					...(certificate ? [certificate] : []),
					...(context.certificates ?? []),
				]),
			],
			...(context.canAssignCertificate ? {} : { lock: "certificates" }),
		},
	];
}

function isolationFields(config: PlacementConfig): SettingField[] {
	const limits = config.resources;
	if (limits?.profile !== "linux_sandbox") return [];
	return [
		{
			id: "cpu",
			section: "isolation",
			kind: "number",
			value: trimmed((limits.cpu_millis ?? 0) / 1000),
			min: 0.1,
			max: 1024,
			step: 0.5,
			unit: "cores",
		},
		{
			id: "mem",
			section: "isolation",
			kind: "number",
			value: trimmed((limits.memory_bytes ?? 0) / GIB),
			min: 0.0625,
			max: 16_384,
			step: 0.5,
			unit: "gib",
		},
		{
			id: "procs",
			section: "isolation",
			kind: "number",
			value: String(limits.max_processes ?? 0),
			min: 16,
			max: 65_536,
			step: 1,
		},
		{
			id: "disk",
			section: "isolation",
			kind: "number",
			value: trimmed((limits.disk_bytes ?? 0) / GIB),
			min: 0.0625,
			max: 1_048_576,
			step: 1,
			unit: "gib",
		},
	];
}

function bufferingFields(config: PlacementConfig): SettingField[] {
	const writes = config.offline_writes;
	if (!writes) return [];
	return [
		{
			id: "qmib",
			section: "buffering",
			kind: "number",
			value: trimmed(writes.max_queue_bytes / MIB),
			min: 1,
			max: 65_536,
			step: 1,
			unit: "mib",
		},
		{
			id: "qage",
			section: "buffering",
			kind: "number",
			value: trimmed(writes.max_age_seconds / DAY_S),
			min: 1,
			max: 30,
			step: 1,
			unit: "days",
		},
	];
}

function instanceLock(
	config: PlacementConfig,
	context: FieldContext,
): FieldLock | undefined {
	if (config.offline_writes) return "buffering_single";
	return context.multiInstance && config.hosting
		? undefined
		: "background_single";
}

/** What Edit settings offers, in sheet order. */
export function settingFields(
	config: PlacementConfig,
	context: FieldContext,
): SettingField[] {
	const lock = instanceLock(config, context);
	return [
		...variableFields(config, context),
		{
			id: "max",
			section: "instances",
			kind: "number",
			value: String(config.max_replicas),
			min: 1,
			max: 32,
			step: 1,
			...(lock ? { lock } : {}),
		},
		...endpointFields(config, context),
		...isolationFields(config),
		...bufferingFields(config),
	];
}

/* Draft → new settings. */

export interface SettingsDraft {
	/** Text the person typed, by field id. */
	values: Readonly<Record<string, string>>;
	/** Variable ids whose stored value goes away (the app's default applies again). */
	removed: readonly string[];
}

export const EMPTY_DRAFT: SettingsDraft = { values: {}, removed: [] };

export type DraftErrorCode =
	| "nothing_changed"
	| "not_a_number"
	| "not_whole"
	| "out_of_range"
	| "invalid_value"
	| "must_resolve"
	| "port_in_use"
	| "port_reserved"
	| "instances_over_approval"
	| "too_large";

export interface DraftError {
	code: DraftErrorCode;
	field?: string;
	params?: Record<string, string | number>;
}

export interface DraftFacts {
	/** Ports other services on the device listen on. */
	otherPorts?: readonly { port: number; service: string }[];
	/** Let's Encrypt on this device answers its challenges on port 80. */
	port80Reserved?: boolean;
	/** Instances the service's cloud approval covers. */
	approvalMaxInstances?: number;
}

export interface DraftResult {
	config: PlacementConfig;
	/** Field ids that differ from the stored settings; `rm.<id>` for removed values. */
	changed: string[];
	errors: DraftError[];
}

type Mutable = Record<string, unknown>;

const WHOLE = new Set(["max", "port", "inflight", "timeout", "procs"]);

const BYTES: Partial<Record<FieldUnit, number>> = { gib: GIB, mib: MIB };

const NUMBER_TARGET: Record<string, (config: Mutable, value: number) => void> =
	{
		max: (config, value) => {
			config.max_replicas = value;
		},
		port: (config, value) => {
			record(config.hosting).port = value;
		},
		inflight: (config, value) => {
			record(config.hosting).max_in_flight = value;
		},
		timeout: (config, value) => {
			record(config.hosting).request_timeout_secs = value;
		},
		cpu: (config, value) => {
			record(config.resources).cpu_millis = Math.round(value * 1000);
		},
		mem: (config, value) => {
			record(config.resources).memory_bytes = Math.round(value * GIB);
		},
		procs: (config, value) => {
			record(config.resources).max_processes = value;
		},
		disk: (config, value) => {
			record(config.resources).disk_bytes = Math.round(value * GIB);
		},
		qmib: (config, value) => {
			record(config.offline_writes).max_queue_bytes = Math.round(value * MIB);
		},
		qage: (config, value) => {
			record(config.offline_writes).max_age_seconds = Math.round(value * DAY_S);
		},
	};

function numberOf(field: SettingField, text: string): number | DraftError {
	const value = Number(text.trim());
	if (text.trim() === "" || !Number.isFinite(value))
		return { code: "not_a_number", field: field.id };
	if (WHOLE.has(field.id) && !Number.isInteger(value))
		return { code: "not_whole", field: field.id };
	const { min = Number.NEGATIVE_INFINITY, max = Number.POSITIVE_INFINITY } =
		field;
	if (value < min || value > max)
		return {
			code: "out_of_range",
			field: field.id,
			params: { value: text.trim(), min, max },
		};
	return value;
}

function variableOf(field: SettingField, text: string): unknown | DraftError {
	const definition = field.variable?.definition;
	const invalid: DraftError = { code: "invalid_value", field: field.id };
	if (definition && !definition.secret) {
		try {
			return { value: variableValue(definition, text) };
		} catch {
			return invalid;
		}
	}
	if (field.kind === "text") return { value: text };
	try {
		const value: unknown = JSON.parse(text);
		return (KIND_FITS[field.kind]?.(value) ?? true) ? { value } : invalid;
	} catch {
		return invalid;
	}
}

/** Without a definition, a stored value keeps the type it has. */
const KIND_FITS: Partial<Record<FieldKind, (value: unknown) => boolean>> = {
	number: (value) => typeof value === "number",
	bool: (value) => typeof value === "boolean",
};

const isError = (value: unknown): value is DraftError =>
	!!value && typeof value === "object" && "code" in value;

interface Edit {
	next: Mutable;
	changed: string[];
	errors: DraftError[];
}

/** Variable ids whose stored value the draft removes. */
type Removed = ReadonlySet<string>;

function editVariable(edit: Edit, field: SettingField, text: string) {
	const id = field.variable?.id ?? "";
	const variables = record(edit.next.variables);
	if (!field.variable?.stored && text === "") return;
	const parsed = variableOf(field, text);
	if (isError(parsed)) {
		edit.errors.push(parsed);
		return;
	}
	variables[id] = (parsed as { value: unknown }).value;
	edit.next.variables = variables;
	edit.changed.push(field.id);
}

function editSelect(edit: Edit, field: SettingField, text: string) {
	if (!field.options?.includes(text)) {
		edit.errors.push({ code: "invalid_value", field: field.id });
		return;
	}
	if (field.id === "bind") record(edit.next.hosting).host = text;
	else if (text === "") Reflect.deleteProperty(edit.next, "tls_certificate_id");
	else edit.next.tls_certificate_id = text;
	edit.changed.push(field.id);
}

function editNumber(edit: Edit, field: SettingField, text: string) {
	const value = numberOf(field, text);
	if (isError(value)) {
		edit.errors.push(value);
		return;
	}
	NUMBER_TARGET[field.id]?.(edit.next, value);
	edit.changed.push(field.id);
}

function editField(edit: Edit, field: SettingField, text: string) {
	if (field.section === "variables") editVariable(edit, field, text);
	else if (field.kind === "select") editSelect(edit, field, text);
	else editNumber(edit, field, text);
}

function removeStored(edit: Edit, fields: readonly SettingField[], id: string) {
	const field = fields.find((entry) => entry.variable?.id === id);
	if (!field?.variable?.stored) return;
	const key = field.kind === "secret" ? "secret_overrides" : "variables";
	const stored = { ...record(edit.next[key]) };
	Reflect.deleteProperty(stored, id);
	edit.next[key] = stored;
	edit.changed.push(`rm.${id}`);
}

/** An incompatible value is settled by a new one; a secret or an unused value only by removing it. */
const replaced = (field: SettingField, edit: Edit) =>
	field.variable?.issue === "incompatible" &&
	field.kind !== "secret" &&
	edit.changed.includes(field.id);

/** A stored value with an issue blocks the change until it is replaced or removed. */
function unresolved(
	fields: readonly SettingField[],
	edit: Edit,
	removed: Removed,
) {
	return fields.flatMap((field): DraftError[] => {
		const variable = field.variable;
		if (!variable?.issue || removed.has(variable.id)) return [];
		if (replaced(field, edit)) return [];
		return [
			{
				code: "must_resolve" as const,
				field: field.id,
				params: { issue: variable.issue },
			},
		];
	});
}

/** The text the draft holds for a field, when it is one the draft may change and it differs. */
function draftedText(
	field: SettingField,
	draft: SettingsDraft,
	removed: Removed,
) {
	if (field.lock || field.kind === "secret") return undefined;
	if (field.variable && removed.has(field.variable.id)) return undefined;
	const text = draft.values[field.id];
	return text === field.value ? undefined : text;
}

function sizeErrors(next: Mutable) {
	const bytes = jsonBytes(next);
	const errors: DraftError[] =
		bytes > CONFIG_MAX_BYTES
			? [{ code: "too_large", params: { bytes, max: CONFIG_MAX_BYTES } }]
			: [];
	return errors;
}

function factErrors(next: Mutable, facts: DraftFacts): DraftError[] {
	const errors: DraftError[] = [];
	const hosting = record(next.hosting);
	const port = hosting.port;
	const clash = facts.otherPorts?.find((other) => other.port === port);
	if (clash)
		errors.push({
			code: "port_in_use",
			field: "port",
			params: { port: clash.port, service: clash.service },
		});
	if (port === 80 && facts.port80Reserved)
		errors.push({ code: "port_reserved", field: "port" });
	const limit = facts.approvalMaxInstances;
	if (limit !== undefined && Number(next.max_replicas) > limit)
		errors.push({
			code: "instances_over_approval",
			field: "max",
			params: { max: limit },
		});
	return errors;
}

/** The stored settings with the draft applied; everything the draft doesn't touch is kept as the device sent it. */
export function applyDraft(
	base: PlacementConfig,
	fields: readonly SettingField[],
	draft: SettingsDraft,
	facts: DraftFacts = {},
): DraftResult {
	const edit: Edit = {
		next: structuredClone(base) as Mutable,
		changed: [],
		errors: [],
	};
	const removed = new Set(draft.removed);
	for (const field of fields) {
		const text = draftedText(field, draft, removed);
		if (text !== undefined) editField(edit, field, text);
	}
	for (const id of removed) removeStored(edit, fields, id);
	edit.errors.push(...unresolved(fields, edit, removed));
	if (edit.changed.some((id) => id === "port" || id === "max"))
		edit.errors.push(...factErrors(edit.next, facts));
	if (!edit.changed.length && !edit.errors.length)
		edit.errors.push({ code: "nothing_changed" });
	edit.errors.push(...sizeErrors(edit.next));
	return {
		config: edit.next as PlacementConfig,
		changed: edit.changed,
		errors: edit.errors,
	};
}

/* Edit as JSON. */

export type JsonErrorCode =
	| "invalid_json"
	| "not_an_object"
	| "identity_changed"
	| "nothing_changed"
	| "too_large";

export interface JsonError {
	code: JsonErrorCode;
	params?: Record<string, string | number>;
}

const IDENTITY = ["id", "project_id", "deployment_id", "source"] as const;

/** The settings a person typed as JSON; the device validates the rest and names what it refuses. */
export function parseJsonSettings(
	base: PlacementConfig,
	text: string,
): { config: PlacementConfig } | { error: JsonError } {
	let value: unknown;
	try {
		value = JSON.parse(text);
	} catch (error) {
		return {
			error: {
				code: "invalid_json",
				params: { detail: error instanceof Error ? error.message : "" },
			},
		};
	}
	if (!value || typeof value !== "object" || Array.isArray(value))
		return { error: { code: "not_an_object" } };
	const next = value as Mutable;
	const moved = IDENTITY.find((key) => next[key] !== base[key]);
	if (moved)
		return {
			error: {
				code: "identity_changed",
				params: { key: moved, value: String(next[moved]) },
			},
		};
	if (JSON.stringify(next) === JSON.stringify(base))
		return { error: { code: "nothing_changed" } };
	const bytes = jsonBytes(next);
	if (bytes > CONFIG_MAX_BYTES)
		return {
			error: { code: "too_large", params: { bytes, max: CONFIG_MAX_BYTES } },
		};
	return { config: next as PlacementConfig };
}

/* What the shared config diff doesn't cover (request to W4-SWITCH: fold into `diffPlacementConfig`). */

export type ExtraDiffField =
	| "limit_parallel"
	| "limit_timeout"
	| "origins"
	| "restart"
	| "other";

export interface ExtraDiffRow {
	kind: "added" | "changed" | "removed";
	field: ExtraDiffField;
	before?: unknown;
	after?: unknown;
}

const COVERED = new Set([
	"revision",
	"online_metadata_sha256",
	"events",
	"variables",
	"secret_overrides",
	"hosting",
	"tls_certificate_id",
	"max_replicas",
	"resource_grant",
	"offline_writes",
	"resources",
	"bit_pins",
	"package_pins",
	"restart",
]);
const COVERED_HOSTING = new Set([
	"host",
	"port",
	"auth_secret",
	"max_in_flight",
	"request_timeout_secs",
	"ui_origins",
]);

const same = (left: unknown, right: unknown) =>
	JSON.stringify(left ?? null) === JSON.stringify(right ?? null);

function extraRow(
	field: ExtraDiffField,
	before: unknown,
	after: unknown,
): ExtraDiffRow[] {
	if (same(before, after)) return [];
	const kind = before == null ? "added" : after == null ? "removed" : "changed";
	return [
		{
			kind,
			field,
			...(before == null ? {} : { before }),
			...(after == null ? {} : { after }),
		},
	];
}

const without = (value: unknown, keys: ReadonlySet<string>) =>
	Object.fromEntries(
		Object.entries(record(value))
			.filter(([key]) => !keys.has(key))
			.sort(([a], [b]) => (a < b ? -1 : 1)),
	);

export function extraDiffRows(
	before: PlacementConfig,
	after: PlacementConfig,
): ExtraDiffRow[] {
	const old = record(before.hosting);
	const next = record(after.hosting);
	const rest = (config: PlacementConfig) => ({
		...without(config, COVERED),
		hosting: without(config.hosting, COVERED_HOSTING),
	});
	return [
		...extraRow("limit_parallel", old.max_in_flight, next.max_in_flight),
		...extraRow(
			"limit_timeout",
			old.request_timeout_secs,
			next.request_timeout_secs,
		),
		...extraRow("origins", old.ui_origins, next.ui_origins),
		...extraRow("restart", record(before).restart, record(after).restart),
		...extraRow("other", rest(before), rest(after)).map(
			(row): ExtraDiffRow => ({ kind: "changed", field: row.field }),
		),
	];
}

/* How a change is applied (IA §6.2 N3: Safe update or Quick update). */

export type ApplyMode = "safe" | "quick";

export interface ApplyHow {
	mode: ApplyMode;
	/** Seconds the new settings must stay healthy. */
	stabilizeS: number;
	/** Seconds the new settings get to start. */
	deadlineS: number;
}

export const DEFAULT_APPLY: ApplyHow = {
	mode: "safe",
	stabilizeS: 10,
	deadlineS: 120,
};

export type ApplyHowError =
	| "stabilize_range"
	| "deadline_range"
	| "deadline_short";

const wholeWithin = (value: number, min: number, max: number) =>
	Number.isInteger(value) && value >= min && value <= max;

export function checkApplyHow(how: ApplyHow): ApplyHowError | null {
	if (how.mode !== "safe") return null;
	const { stabilizeS, deadlineS } = how;
	if (!wholeWithin(stabilizeS, 2, 60)) return "stabilize_range";
	if (!wholeWithin(deadlineS, 10, 600)) return "deadline_range";
	return deadlineS <= stabilizeS + 5 ? "deadline_short" : null;
}

export class ConfigTooLargeError extends Error {
	constructor(readonly bytes: number) {
		super(
			`The settings need ${bytes} bytes; one message to the device carries at most ${CONFIG_MAX_BYTES}.`,
		);
		this.name = "ConfigTooLargeError";
	}
}

/**
 * The commands for one settings change. A quick update keeps the requested
 * state (a running service restarts with the new settings, a stopped one
 * stays stopped); a safe update stages the settings and activates them.
 * No secret is part of it: stored references travel unchanged.
 */
export function settingsPlan(
	existing: Pick<PlacementConfiguration, "config_revision">,
	config: PlacementConfig,
	how: ApplyHow,
	running: boolean,
): DeploymentPlan {
	const expected = existing.config_revision;
	const first = crypto.randomUUID();
	const steps: DeploymentPlan["steps"] =
		how.mode === "safe"
			? [
					{
						id: first,
						command: {
							type: "stage_rollout",
							config,
							expected_revision: expected,
							stabilization_seconds: how.stabilizeS,
							deadline_seconds: how.deadlineS,
						},
					},
					{
						id: crypto.randomUUID(),
						command: { type: "activate_rollout", rollout_id: first },
					},
				]
			: [
					{
						id: first,
						command: {
							type: "apply",
							config,
							expected_revision: expected,
							start: running,
						},
					},
				];
	const largest = Math.max(...steps.map((step) => jsonBytes(step.command)));
	if (largest > CONFIG_MAX_BYTES) throw new ConfigTooLargeError(largest);
	return {
		config,
		steps,
		expected_revision: expected,
		...(how.mode === "safe" ? { rollout_id: first } : {}),
	};
}

/* Queue targets (Write buffering). */

export type QueueTarget =
	| { kind: "table"; name: string; purpose: string }
	| { kind: "file"; name: string; purpose: string }
	| null;

/** What a buffered change writes to, from the device's resource descriptor. */
export function queueTarget(resource: string): QueueTarget {
	let parsed: unknown;
	try {
		parsed = JSON.parse(resource);
	} catch {
		return null;
	}
	const value = record(parsed);
	const purpose = isText(value.purpose) ? value.purpose : "";
	if (value.kind === "table" && isText(value.table))
		return { kind: "table", name: value.table, purpose };
	if (value.kind === "file" && isText(value.path))
		return { kind: "file", name: value.path, purpose };
	return null;
}

export const HEAD_NEEDS_YOU = new Set([
	"blocked",
	"conflict",
	"outcome_unknown",
]);
