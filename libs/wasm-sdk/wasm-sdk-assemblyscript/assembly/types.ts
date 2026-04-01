import { packResult } from "./host";
import {
	jsonString,
	serializeBoolArray,
	serializeF64Array,
	serializeI64Array,
	serializeStringArray,
	serializeStringMap,
} from "./json";

export const ABI_VERSION: i32 = 1;

export enum LogLevel {
	Debug = 0,
	Info = 1,
	Warn = 2,
	Error = 3,
	Fatal = 4,
}

export namespace DataType {
	export const Exec: string = "Exec";
	export const String: string = "String";
	export const I64: string = "I64";
	export const F64: string = "F64";
	export const Bool: string = "Bool";
	export const Date: string = "Date";
	export const PathBuf: string = "PathBuf";
	export const Bytes: string = "Bytes";
	export const Struct: string = "Struct";
	export const Generic: string = "Generic";
}

export namespace ValueType {
	export const Normal: string = "Normal";
	export const Array: string = "Array";
	export const HashMap: string = "HashMap";
	export const HashSet: string = "HashSet";
}

export class NodeScores {
	privacy: u8 = 0;
	security: u8 = 0;
	performance: u8 = 0;
	governance: u8 = 0;
	reliability: u8 = 0;
	cost: u8 = 0;

	toJSON(): string {
		return `{"privacy":${this.privacy},"security":${this.security},"performance":${this.performance},"governance":${this.governance},"reliability":${this.reliability},"cost":${this.cost}}`;
	}
}

export class PinDefinition {
	name = "";
	friendly_name = "";
	description = "";
	pin_type = "";
	data_type = "";
	default_value: string | null = null;
	value_type: string | null = null;
	schema: string | null = null;
	step: f64 = 0;
	has_step: bool = false;
	sensitive: bool = false;
	has_sensitive: bool = false;
	enforce_schema_flag: bool = false;
	has_enforce_schema: bool = false;
	enforce_generic_value_type: bool = false;
	has_enforce_generic_value_type: bool = false;

	static input(
		name: string,
		friendlyName: string,
		description: string,
		dataType: string,
	): PinDefinition {
		const pin = new PinDefinition();
		pin.name = name;
		pin.friendly_name = friendlyName;
		pin.description = description;
		pin.pin_type = "Input";
		pin.data_type = dataType;
		return pin;
	}

	static output(
		name: string,
		friendlyName: string,
		description: string,
		dataType: string,
	): PinDefinition {
		const pin = new PinDefinition();
		pin.name = name;
		pin.friendly_name = friendlyName;
		pin.description = description;
		pin.pin_type = "Output";
		pin.data_type = dataType;
		return pin;
	}

	withDefault(value: string): PinDefinition {
		this.default_value = value;
		return this;
	}

	withDefaultString(value: string): PinDefinition {
		this.default_value = jsonString(value);
		return this;
	}

	withDefaultI64(value: i64): PinDefinition {
		this.default_value = value.toString();
		return this;
	}

	withDefaultF64(value: f64): PinDefinition {
		this.default_value = value.toString();
		return this;
	}

	withDefaultBool(value: bool): PinDefinition {
		this.default_value = value ? "true" : "false";
		return this;
	}

	withDefaultStringArray(values: string[]): PinDefinition {
		this.default_value = serializeStringArray(values);
		this.value_type = ValueType.Array;
		return this;
	}

	withDefaultI64Array(values: i64[]): PinDefinition {
		this.default_value = serializeI64Array(values);
		this.value_type = ValueType.Array;
		return this;
	}

	withDefaultF64Array(values: f64[]): PinDefinition {
		this.default_value = serializeF64Array(values);
		this.value_type = ValueType.Array;
		return this;
	}

	withDefaultBoolArray(values: bool[]): PinDefinition {
		this.default_value = serializeBoolArray(values);
		this.value_type = ValueType.Array;
		return this;
	}

	withDefaultStringMap(values: Map<string, string>): PinDefinition {
		this.default_value = serializeStringMap(values);
		this.value_type = ValueType.HashMap;
		return this;
	}

	asArray(): PinDefinition {
		this.value_type = ValueType.Array;
		return this;
	}

	asHashMap(): PinDefinition {
		this.value_type = ValueType.HashMap;
		return this;
	}

	asHashSet(): PinDefinition {
		this.value_type = ValueType.HashSet;
		return this;
	}

	withSchema(schema: string): PinDefinition {
		this.schema = schema;
		return this;
	}

	withStep(step: f64): PinDefinition {
		this.step = step;
		this.has_step = true;
		return this;
	}

	withSensitive(sensitive: bool = true): PinDefinition {
		this.sensitive = sensitive;
		this.has_sensitive = true;
		return this;
	}

	withEnforceSchema(enforce: bool = true): PinDefinition {
		this.enforce_schema_flag = enforce;
		this.has_enforce_schema = true;
		return this;
	}

	withEnforceGenericValueType(enforce: bool = true): PinDefinition {
		this.enforce_generic_value_type = enforce;
		this.has_enforce_generic_value_type = true;
		return this;
	}

	toJSON(): string {
		let json = `{"name":${jsonString(this.name)},"friendly_name":${jsonString(this.friendly_name)},"description":${jsonString(this.description)},"pin_type":"${this.pin_type}","data_type":"${this.data_type}"`;
		if (this.default_value !== null)
			json += `,"default_value":${this.default_value!}`;
		if (this.value_type !== null)
			json += `,"value_type":${jsonString(this.value_type!)}`;
		if (this.schema !== null) json += `,"schema":${jsonString(this.schema!)}`;
		if (this.has_step) json += `,"step":${this.step.toString()}`;
		if (this.has_sensitive) json += `,"sensitive":${this.sensitive ? "true" : "false"}`;
		if (this.has_enforce_schema) json += `,"enforce_schema":${this.enforce_schema_flag ? "true" : "false"}`;
		if (this.has_enforce_generic_value_type) json += `,"enforce_generic_value_type":${this.enforce_generic_value_type ? "true" : "false"}`;
		json += "}";
		return json;
	}
}

export class NodeDefinition {
	name = "";
	friendly_name = "";
	description = "";
	category = "";
	icon: string | null = null;
	pins: PinDefinition[] = [];
	scores: NodeScores | null = null;
	long_running: bool = false;
	docs: string | null = null;
	abi_version: i32 = ABI_VERSION;
	permissions: string[] = [];

	addPin(pin: PinDefinition): NodeDefinition {
		this.pins.push(pin);
		return this;
	}

	setScores(scores: NodeScores): NodeDefinition {
		this.scores = scores;
		return this;
	}

	addPermission(permission: string): NodeDefinition {
		this.permissions.push(permission);
		return this;
	}

	toJSON(): string {
		let pinsJson = "[";
		for (let i = 0; i < this.pins.length; i++) {
			if (i > 0) pinsJson += ",";
			pinsJson += this.pins[i].toJSON();
		}
		pinsJson += "]";

		let json = `{"name":${jsonString(this.name)},"friendly_name":${jsonString(this.friendly_name)},"description":${jsonString(this.description)},"category":${jsonString(this.category)},"pins":${pinsJson},"long_running":${this.long_running},"abi_version":${this.abi_version}`;
		if (this.icon !== null) json += `,"icon":${jsonString(this.icon!)}`;
		if (this.scores !== null) json += `,"scores":${this.scores!.toJSON()}`;
		if (this.docs !== null) json += `,"docs":${jsonString(this.docs!)}`;
		if (this.permissions.length > 0) {
			json += `,"permissions":[`;
			for (let i = 0; i < this.permissions.length; i++) {
				if (i > 0) json += ",";
				json += jsonString(this.permissions[i]);
			}
			json += "]";
		}
		json += "}";
		return json;
	}
}

export class PackageNodes {
	nodes: NodeDefinition[] = [];

	addNode(node: NodeDefinition): PackageNodes {
		this.nodes.push(node);
		return this;
	}

	toJSON(): string {
		let json = "[";
		for (let i = 0; i < this.nodes.length; i++) {
			if (i > 0) json += ",";
			json += this.nodes[i].toJSON();
		}
		json += "]";
		return json;
	}

	toWasm(): i64 {
		return packResult(this.toJSON());
	}
}

export class ExecutionInput {
	inputs: Map<string, string> = new Map();
	node_id = "";
	node_name = "";
	run_id = "";
	app_id = "";
	board_id = "";
	user_id = "";
	stream_state: bool = false;
	log_level: u8 = 1;
}

export class ExecutionResult {
	outputs: Map<string, string> = new Map();
	error: string | null = null;
	activate_exec: string[] = [];
	pending: bool = false;

	static success(): ExecutionResult {
		return new ExecutionResult();
	}

	static fail(message: string): ExecutionResult {
		const result = new ExecutionResult();
		result.error = message;
		return result;
	}

	setOutput(name: string, value: string): ExecutionResult {
		this.outputs.set(name, value);
		return this;
	}

	setOutputString(name: string, value: string): ExecutionResult {
		this.outputs.set(name, jsonString(value));
		return this;
	}

	setOutputI64(name: string, value: i64): ExecutionResult {
		this.outputs.set(name, value.toString());
		return this;
	}

	setOutputF64(name: string, value: f64): ExecutionResult {
		this.outputs.set(name, value.toString());
		return this;
	}

	setOutputBool(name: string, value: bool): ExecutionResult {
		this.outputs.set(name, value ? "true" : "false");
		return this;
	}

	setOutputStringArray(name: string, values: string[]): ExecutionResult {
		this.outputs.set(name, serializeStringArray(values));
		return this;
	}

	setOutputI64Array(name: string, values: i64[]): ExecutionResult {
		this.outputs.set(name, serializeI64Array(values));
		return this;
	}

	setOutputF64Array(name: string, values: f64[]): ExecutionResult {
		this.outputs.set(name, serializeF64Array(values));
		return this;
	}

	setOutputBoolArray(name: string, values: bool[]): ExecutionResult {
		this.outputs.set(name, serializeBoolArray(values));
		return this;
	}

	setOutputStringMap(
		name: string,
		values: Map<string, string>,
	): ExecutionResult {
		this.outputs.set(name, serializeStringMap(values));
		return this;
	}

	activateExec(pinName: string): ExecutionResult {
		this.activate_exec.push(pinName);
		return this;
	}

	setPending(pending: bool): ExecutionResult {
		this.pending = pending;
		return this;
	}

	toJSON(): string {
		let outputsJson = "{";
		const keys = this.outputs.keys();
		for (let i = 0; i < keys.length; i++) {
			if (i > 0) outputsJson += ",";
			const key = keys[i];
			outputsJson += `${jsonString(key)}:${this.outputs.get(key)}`;
		}
		outputsJson += "}";

		let execJson = "[";
		for (let i = 0; i < this.activate_exec.length; i++) {
			if (i > 0) execJson += ",";
			execJson += jsonString(this.activate_exec[i]);
		}
		execJson += "]";

		let json = `{"outputs":${outputsJson},"activate_exec":${execJson},"pending":${this.pending}`;
		if (this.error !== null) json += `,"error":${jsonString(this.error!)}`;
		json += "}";
		return json;
	}
}
