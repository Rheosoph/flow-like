export const ABI_VERSION = 1;

export enum LogLevel {
	DEBUG = 0,
	INFO = 1,
	WARN = 2,
	ERROR = 3,
	FATAL = 4,
}

export namespace PinType {
	export const EXEC = "Exec";
	export const STRING = "String";
	export const I64 = "I64";
	export const F64 = "F64";
	export const BOOL = "Bool";
	export const GENERIC = "Generic";
	export const BYTES = "Bytes";
	export const DATE = "Date";
	export const PATH_BUF = "PathBuf";
	export const STRUCT = "Struct";

	const ALL = new Set([
		EXEC,
		STRING,
		I64,
		F64,
		BOOL,
		GENERIC,
		BYTES,
		DATE,
		PATH_BUF,
		STRUCT,
	]);

	export function validate(dataType: string): string {
		if (!ALL.has(dataType)) {
			throw new Error(
				`Invalid pin data type: ${dataType}. Must be one of ${[...ALL].join(", ")}`,
			);
		}
		return dataType;
	}
}

/** Alias: prefer DataType over PinType for clarity (in core, PinType means Input/Output). */
export

export namespace ValueType {
	export const NORMAL = "Normal";
	export const ARRAY = "Array";
	export const HASH_MAP = "HashMap";
	export const HASH_SET = "HashSet";

	const ALL = new Set([NORMAL, ARRAY, HASH_MAP, HASH_SET]);

	export function validate(valueType: string): string {
		if (!ALL.has(valueType)) {
			throw new Error(
				`Invalid value type: ${valueType}. Must be one of ${[...ALL].join(", ")}`,
			);
		}
		return valueType;
	}
}

export function humanize(name: string): string {
	return name
		.split("_")
		.filter(Boolean)
		.map((w) => w.charAt(0).toUpperCase() + w.slice(1))
		.join(" ");
}

export interface NodeScoresData {
	readonly privacy: number;
	readonly security: number;
	readonly performance: number;
	readonly governance: number;
	readonly reliability: number;
	readonly cost: number;
}

export class NodeScores implements NodeScoresData {
	readonly privacy: number;
	readonly security: number;
	readonly performance: number;
	readonly governance: number;
	readonly reliability: number;
	readonly cost: number;

	constructor(data: Partial<NodeScoresData> = {}) {
		this.privacy = data.privacy ?? 0;
		this.security = data.security ?? 0;
		this.performance = data.performance ?? 0;
		this.governance = data.governance ?? 0;
		this.reliability = data.reliability ?? 0;
		this.cost = data.cost ?? 0;
	}

	toDict(): Record<string, number> {
		return {
			privacy: this.privacy,
			security: this.security,
			performance: this.performance,
			governance: this.governance,
			reliability: this.reliability,
			cost: this.cost,
		};
	}
}

export class PinDefinition {
	name: string;
	friendlyName: string;
	description: string;
	pinType: string;
	dataType: string;
	defaultValue: unknown;
	valueType: string | null;
	schema: string | null;
	validValues: string[] | null;
	range: [number, number] | null;
	step: number | null;
	sensitive: boolean | null;
	enforceSchema: boolean | null;
	enforceGenericValueType: boolean | null;

	private constructor(
		name: string,
		friendlyName: string,
		description: string,
		pinType: string,
		dataType: string,
		defaultValue: unknown = null,
	) {
		this.name = name;
		this.friendlyName = friendlyName;
		this.description = description;
		this.pinType = pinType;
		this.dataType = dataType;
		this.defaultValue = defaultValue;
		this.valueType = null;
		this.schema = null;
		this.validValues = null;
		this.range = null;
		this.step = null;
		this.sensitive = null;
		this.enforceSchema = null;
		this.enforceGenericValueType = null;
	}

	static inputPin(
		name: string,
		dataType: string,
		options: {
			description?: string;
			defaultValue?: unknown;
			friendlyName?: string;
		} = {},
	): PinDefinition {
		PinType.validate(dataType);
		return new PinDefinition(
			name,
			options.friendlyName ?? humanize(name),
			options.description ?? `Input: ${name}`,
			"Input",
			dataType,
			options.defaultValue ?? null,
		);
	}

	static outputPin(
		name: string,
		dataType: string,
		options: { description?: string; friendlyName?: string } = {},
	): PinDefinition {
		PinType.validate(dataType);
		return new PinDefinition(
			name,
			options.friendlyName ?? humanize(name),
			options.description ?? `Output: ${name}`,
			"Output",
			dataType,
		);
	}

	static inputExec(name = "exec", description = ""): PinDefinition {
		return new PinDefinition(
			name,
			humanize(name),
			description || `Input: ${name}`,
			"Input",
			PinType.EXEC,
		);
	}

	static outputExec(name = "exec_out", description = ""): PinDefinition {
		return new PinDefinition(
			name,
			humanize(name),
			description || `Output: ${name}`,
			"Output",
			PinType.EXEC,
		);
	}

	withDefault(value: unknown): this {
		this.defaultValue = value;
		return this;
	}

	withValueType(valueType: string): this {
		ValueType.validate(valueType);
		this.valueType = valueType;
		return this;
	}

	withSchema(schema: string): this {
		this.schema = schema;
		return this;
	}

	/**
	 * Derive a JSON Schema from a TypeBox `TSchema` and attach it to this pin.
	 *
	 * @example
	 * ```ts
	 * import { Type } from "@sinclair/typebox";
	 *
	 * const Config = Type.Object({ threshold: Type.Number(), label: Type.String() });
	 *
	 * const pin = PinDefinition.inputPin("config", PinType.STRUCT)
	 *   .withSchemaType(Config);
	 * ```
	 */
	withSchemaType(schema: import("@sinclair/typebox").TSchema): this {
		this.schema = JSON.stringify(schema);
		return this;
	}

	withValidValues(values: string[]): this {
		this.validValues = values;
		return this;
	}

	withRange(min: number, max: number): this {
		this.range = [min, max];
		return this;
	}

	withStep(step: number): this {
		this.step = step;
		return this;
	}

	withSensitive(sensitive = true): this {
		this.sensitive = sensitive;
		return this;
	}

	withEnforceSchema(enforce = true): this {
		this.enforceSchema = enforce;
		return this;
	}

	withEnforceGenericValueType(enforce = true): this {
		this.enforceGenericValueType = enforce;
		return this;
	}

	toDict(): Record<string, unknown> {
		const d: Record<string, unknown> = {
			name: this.name,
			friendly_name: this.friendlyName,
			description: this.description,
			pin_type: this.pinType,
			data_type: this.dataType,
		};
		if (this.defaultValue != null) d.default_value = this.defaultValue;
		if (this.valueType != null) d.value_type = this.valueType;
		if (this.schema != null) d.schema = this.schema;
		if (this.validValues != null) d.valid_values = this.validValues;
		if (this.range != null) d.range = this.range;
		if (this.step != null) d.step = this.step;
		if (this.sensitive != null) d.sensitive = this.sensitive;
		if (this.enforceSchema != null) d.enforce_schema = this.enforceSchema;
		if (this.enforceGenericValueType != null)
			d.enforce_generic_value_type = this.enforceGenericValueType;
		return d;
	}
}

export class NodeDefinition {
	name: string;
	friendlyName: string;
	description: string;
	category: string;
	icon: string | null;
	pins: PinDefinition[];
	scores: NodeScores | null;
	longRunning: boolean | null;
	docs: string | null;
	permissions: string[];
	abiVersion: number;

	constructor(
		name: string,
		friendlyName: string,
		description: string,
		category: string,
		options: { icon?: string; docs?: string } = {},
	) {
		this.name = name;
		this.friendlyName = friendlyName;
		this.description = description;
		this.category = category;
		this.icon = options.icon ?? null;
		this.pins = [];
		this.scores = null;
		this.longRunning = null;
		this.docs = options.docs ?? null;
		this.permissions = [];
		this.abiVersion = ABI_VERSION;
	}

	addPin(pin: PinDefinition): this {
		this.pins.push(pin);
		return this;
	}

	setScores(scores: NodeScores): this {
		this.scores = scores;
		return this;
	}

	setLongRunning(longRunning: boolean): this {
		this.longRunning = longRunning;
		return this;
	}

	addPermission(permission: string): this {
		this.permissions.push(permission);
		return this;
	}

	toDict(): Record<string, unknown> {
		const d: Record<string, unknown> = {
			name: this.name,
			friendly_name: this.friendlyName,
			description: this.description,
			category: this.category,
			pins: this.pins.map((p) => p.toDict()),
			abi_version: this.abiVersion,
		};
		if (this.icon != null) d.icon = this.icon;
		if (this.scores != null) d.scores = this.scores.toDict();
		if (this.longRunning != null) d.long_running = this.longRunning;
		if (this.docs != null) d.docs = this.docs;
		if (this.permissions.length > 0) d.permissions = this.permissions;
		return d;
	}

	toJSON(): string {
		return JSON.stringify(this.toDict());
	}
}

export class PackageNodes {
	readonly nodes: NodeDefinition[] = [];

	addNode(node: NodeDefinition): this {
		this.nodes.push(node);
		return this;
	}

	toDict(): Record<string, unknown>[] {
		return this.nodes.map((n) => n.toDict());
	}

	toJSON(): string {
		return JSON.stringify(this.toDict());
	}
}

export interface ExecutionInputData {
	readonly inputs: Record<string, unknown>;
	readonly node_id: string;
	readonly run_id: string;
	readonly app_id: string;
	readonly board_id: string;
	readonly user_id: string;
	readonly stream_state: boolean;
	readonly log_level: LogLevel;
	readonly node_name: string;
}

export class ExecutionInput {
	readonly inputs: Record<string, unknown>;
	readonly nodeId: string;
	readonly runId: string;
	readonly appId: string;
	readonly boardId: string;
	readonly userId: string;
	readonly streamState: boolean;
	readonly logLevel: LogLevel;
	readonly nodeName: string;

	private constructor(data: Partial<ExecutionInputData>) {
		this.inputs = data.inputs ?? {};
		this.nodeId = data.node_id ?? "";
		this.runId = data.run_id ?? "";
		this.appId = data.app_id ?? "";
		this.boardId = data.board_id ?? "";
		this.userId = data.user_id ?? "";
		this.streamState = data.stream_state ?? false;
		this.logLevel = data.log_level ?? LogLevel.INFO;
		this.nodeName = data.node_name ?? "";
	}

	static fromDict(data: Record<string, unknown>): ExecutionInput {
		return new ExecutionInput(data as Partial<ExecutionInputData>);
	}

	static fromJSON(json: string): ExecutionInput {
		return ExecutionInput.fromDict(JSON.parse(json));
	}
}

export class ExecutionResult {
	readonly outputs: Record<string, unknown>;
	error: string | null;
	readonly activateExec: string[];
	pending: boolean | null;

	private constructor(error: string | null = null) {
		this.outputs = {};
		this.error = error;
		this.activateExec = [];
		this.pending = null;
	}

	static ok(): ExecutionResult {
		return new ExecutionResult();
	}

	static fail(message: string): ExecutionResult {
		return new ExecutionResult(message);
	}

	setOutput(name: string, value: unknown): this {
		this.outputs[name] = value;
		return this;
	}

	exec(pinName: string): this {
		this.activateExec.push(pinName);
		return this;
	}

	setPending(pending: boolean): this {
		this.pending = pending;
		return this;
	}

	toDict(): Record<string, unknown> {
		const d: Record<string, unknown> = {
			outputs: this.outputs,
			activate_exec: this.activateExec,
		};
		if (this.error != null) d.error = this.error;
		if (this.pending != null) d.pending = this.pending;
		return d;
	}

	toJSON(): string {
		return JSON.stringify(this.toDict());
	}
}
