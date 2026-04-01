import {
	stream,
	getVariable as hostGetVariable,
	setVariable as hostSetVariable,
	streamText as hostStreamText,
	debug as logDebug,
	error as logError,
	info as logInfo,
	warn as logWarn,
} from "./host";
import {
	jsonString,
	parseJsonBoolArray,
	parseJsonF64Array,
	parseJsonI64Array,
	parseJsonStringArray,
	parseJsonStringMap,
	serializeBoolArray,
	serializeF64Array,
	serializeI64Array,
	serializeStringArray,
	serializeStringMap,
} from "./json";
import { type ExecutionInput, ExecutionResult, LogLevel } from "./types";

export class Context {
	private _input: ExecutionInput;
	private _result: ExecutionResult;
	private _outputs: Map<string, string>;

	constructor(input: ExecutionInput) {
		this._input = input;
		this._result = ExecutionResult.success();
		this._outputs = new Map();
	}

	// -- Metadata --

	get nodeId(): string {
		return this._input.node_id;
	}
	get nodeName(): string {
		return this._input.node_name;
	}
	get runId(): string {
		return this._input.run_id;
	}
	get appId(): string {
		return this._input.app_id;
	}
	get boardId(): string {
		return this._input.board_id;
	}
	get userId(): string {
		return this._input.user_id;
	}
	get streamEnabled(): bool {
		return this._input.stream_state;
	}
	get logLevel(): u8 {
		return this._input.log_level;
	}

	// -- Input getters --

	getInput(name: string): string | null {
		return this._input.inputs.has(name) ? this._input.inputs.get(name) : null;
	}

	getString(name: string, defaultValue = ""): string {
		const val = this.getInput(name);
		if (val === null) return defaultValue;
		const v = val!;
		if (v.startsWith('"') && v.endsWith('"')) return v.slice(1, v.length - 1);
		return v;
	}

	getI64(name: string, defaultValue: i64 = 0): i64 {
		const val = this.getInput(name);
		if (val === null) return defaultValue;
		return I64.parseInt(val!);
	}

	getF64(name: string, defaultValue: f64 = 0): f64 {
		const val = this.getInput(name);
		if (val === null) return defaultValue;
		return F64.parseFloat(val!);
	}

	getBool(name: string, defaultValue: bool = false): bool {
		const val = this.getInput(name);
		if (val === null) return defaultValue;
		return val! == "true";
	}

	requireInput(name: string): string {
		const val = this.getInput(name);
		if (val === null) throw new Error("Missing required input: " + name);
		return val!;
	}

	// -- Array input getters --

	getStringArray(name: string): string[] {
		const val = this.getInput(name);
		if (val === null) return [];
		return parseJsonStringArray(val!);
	}

	getI64Array(name: string): i64[] {
		const val = this.getInput(name);
		if (val === null) return [];
		return parseJsonI64Array(val!);
	}

	getF64Array(name: string): f64[] {
		const val = this.getInput(name);
		if (val === null) return [];
		return parseJsonF64Array(val!);
	}

	getBoolArray(name: string): bool[] {
		const val = this.getInput(name);
		if (val === null) return [];
		return parseJsonBoolArray(val!);
	}

	// -- Map input getters --

	getStringMap(name: string): Map<string, string> {
		const val = this.getInput(name);
		if (val === null) return new Map<string, string>();
		return parseJsonStringMap(val!);
	}

	// -- Output setters --

	setOutput(name: string, value: string): void {
		this._outputs.set(name, value);
	}

	setString(name: string, value: string): void {
		this._outputs.set(name, jsonString(value));
	}

	setI64(name: string, value: i64): void {
		this._outputs.set(name, value.toString());
	}

	setF64(name: string, value: f64): void {
		this._outputs.set(name, value.toString());
	}

	setBool(name: string, value: bool): void {
		this._outputs.set(name, value ? "true" : "false");
	}

	// -- Array output setters --

	setStringArray(name: string, values: string[]): void {
		this._outputs.set(name, serializeStringArray(values));
	}

	setI64Array(name: string, values: i64[]): void {
		this._outputs.set(name, serializeI64Array(values));
	}

	setF64Array(name: string, values: f64[]): void {
		this._outputs.set(name, serializeF64Array(values));
	}

	setBoolArray(name: string, values: bool[]): void {
		this._outputs.set(name, serializeBoolArray(values));
	}

	// -- Map output setters --

	setStringMap(name: string, values: Map<string, string>): void {
		this._outputs.set(name, serializeStringMap(values));
	}

	activateExec(pinName: string): void {
		this._result.activate_exec.push(pinName);
	}

	setPending(pending: bool): void {
		this._result.pending = pending;
	}

	setError(error: string): void {
		this._result.error = error;
	}

	// -- Level-gated logging --

	shouldLog(level: LogLevel): bool {
		return level >= this._input.log_level;
	}

	debug(message: string): void {
		if (this.shouldLog(LogLevel.Debug)) logDebug(message);
	}

	info(message: string): void {
		if (this.shouldLog(LogLevel.Info)) logInfo(message);
	}

	warn(message: string): void {
		if (this.shouldLog(LogLevel.Warn)) logWarn(message);
	}

	error(message: string): void {
		if (this.shouldLog(LogLevel.Error)) logError(message);
	}

	// -- Conditional streaming --

	streamText(text: string): void {
		if (this.streamEnabled) hostStreamText(text);
	}

	streamJson(data: string): void {
		if (this.streamEnabled) stream("json", data);
	}

	streamProgress(progress: f32, message: string): void {
		if (this.streamEnabled)
			stream("progress", `{"progress":${progress},"message":"${message}"}`);
	}

	// -- Variables --

	getVariable(name: string): string | null {
		return hostGetVariable(name);
	}

	setVariable(name: string, value: string): void {
		hostSetVariable(name, value);
	}

	// -- Finalize --

	finish(): ExecutionResult {
		const keys = this._outputs.keys();
		for (let i = 0; i < keys.length; i++) {
			this._result.setOutput(keys[i], this._outputs.get(keys[i]));
		}
		return this._result;
	}

	success(): ExecutionResult {
		this.activateExec("exec_out");
		return this.finish();
	}

	fail(error: string): ExecutionResult {
		this.setError(error);
		return this.finish();
	}
}
