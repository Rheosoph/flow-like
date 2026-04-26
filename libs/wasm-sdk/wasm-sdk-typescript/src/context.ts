import type { FlowPath } from "./host";
import { type HostBridge, getHost } from "./host";
import { ExecutionInput, ExecutionResult, LogLevel } from "./types";

const CHUNK_SIZE = 8 * 1024 * 1024; // 8 MiB, safely under host 10 MiB limit

export class Context {
	private _input: ExecutionInput;
	private _result: ExecutionResult;
	private _host: HostBridge;

	constructor(executionInput: ExecutionInput, host?: HostBridge) {
		this._input = executionInput;
		this._result = ExecutionResult.ok();
		this._host = host ?? getHost();
	}

	static fromDict(data: Record<string, unknown>, host?: HostBridge): Context {
		return new Context(ExecutionInput.fromDict(data), host);
	}

	static fromJSON(jsonStr: string, host?: HostBridge): Context {
		return new Context(ExecutionInput.fromJSON(jsonStr), host);
	}

	// Metadata getters
	get nodeId(): string {
		return this._input.nodeId;
	}
	get nodeName(): string {
		return this._input.nodeName;
	}
	get runId(): string {
		return this._input.runId;
	}
	get appId(): string {
		return this._input.appId;
	}
	get boardId(): string {
		return this._input.boardId;
	}
	get userId(): string {
		return this._input.userId;
	}
	get streamEnabled(): boolean {
		return this._input.streamState;
	}
	get logLevel(): number {
		return this._input.logLevel;
	}

	// Input getters
	getInput(name: string): unknown {
		return this._input.inputs[name] ?? null;
	}

	getString(name: string, defaultValue?: string): string | undefined {
		const val = this.getInput(name);
		if (val == null) return defaultValue;
		return String(val);
	}

	getI64(name: string, defaultValue?: number): number | undefined {
		const val = this.getInput(name);
		if (val == null) return defaultValue;
		return Number(val);
	}

	getF64(name: string, defaultValue?: number): number | undefined {
		const val = this.getInput(name);
		if (val == null) return defaultValue;
		return Number(val);
	}

	getBool(name: string, defaultValue?: boolean): boolean | undefined {
		const val = this.getInput(name);
		if (val == null) return defaultValue;
		return Boolean(val);
	}

	requireInput(name: string): unknown {
		const val = this.getInput(name);
		if (val == null) throw new Error(`Required input '${name}' not provided`);
		return val;
	}

	// Output setters
	setOutput(name: string, value: unknown): void {
		this._result.setOutput(name, value);
	}

	activateExec(pinName: string): void {
		this._result.exec(pinName);
	}

	setPending(pending: boolean): void {
		this._result.setPending(pending);
	}

	// Level-gated logging
	debug(message: string): void {
		if (this._input.logLevel <= LogLevel.DEBUG)
			this._host.log(LogLevel.DEBUG, message);
	}

	info(message: string): void {
		if (this._input.logLevel <= LogLevel.INFO)
			this._host.log(LogLevel.INFO, message);
	}

	warn(message: string): void {
		if (this._input.logLevel <= LogLevel.WARN)
			this._host.log(LogLevel.WARN, message);
	}

	error(message: string): void {
		if (this._input.logLevel <= LogLevel.ERROR)
			this._host.log(LogLevel.ERROR, message);
	}

	// Streaming (only when enabled)
	streamText(text: string): void {
		if (this._input.streamState) this._host.stream("text", text);
	}

	streamJSON(data: unknown): void {
		if (this._input.streamState)
			this._host.stream("json", JSON.stringify(data));
	}

	streamProgress(progress: number, message: string): void {
		if (this._input.streamState) {
			this._host.stream("progress", JSON.stringify({ progress, message }));
		}
	}

	// Variables
	getVariable(name: string): unknown {
		return this._host.getVariable(name);
	}

	setVariable(name: string, value: unknown): boolean {
		return this._host.setVariable(name, value);
	}

	// Storage
	storageDir(nodeScoped = false): FlowPath | null {
		return this._host.storageDir(nodeScoped);
	}

	uploadDir(): FlowPath | null {
		return this._host.uploadDir();
	}

	cacheDir(nodeScoped = false, userScoped = false): FlowPath | null {
		return this._host.cacheDir(nodeScoped, userScoped);
	}

	userDir(nodeScoped = false): FlowPath | null {
		return this._host.userDir(nodeScoped);
	}

	storageRead(flowPath: FlowPath): Uint8Array | null {
		return this._host.storageRead(flowPath);
	}

	storageWrite(flowPath: FlowPath, data: Uint8Array): boolean {
		if (data.byteLength <= CHUNK_SIZE) {
			return this._host.storageWrite(flowPath, data);
		}
		const writeId = this._host.storageWriteStart(flowPath, data.byteLength);
		if (writeId === null) return false;
		for (let offset = 0; offset < data.byteLength; offset += CHUNK_SIZE) {
			const chunk = data.subarray(
				offset,
				Math.min(offset + CHUNK_SIZE, data.byteLength),
			);
			if (!this._host.storageWriteChunk(writeId, chunk)) return false;
		}
		return this._host.storageWriteFinish(writeId);
	}

	storageList(flowPath: FlowPath): FlowPath[] | null {
		return this._host.storageList(flowPath);
	}

	embedText(bit: Record<string, unknown>, texts: string[]): number[][] | null {
		return this._host.embedText(bit, texts);
	}

	httpRequest(
		method: number,
		url: string,
		headers: Record<string, string> = {},
		body: Uint8Array | null = null,
	): { status: number; headers: Record<string, string>; body: string } | null {
		const result = this._host.httpRequest(
			method,
			url,
			JSON.stringify(headers),
			body,
		);
		if (!result) return null;
		const parsed = JSON.parse(result);
		if (parsed.body_base64 && !parsed.body) {
			try {
				const bytes = Uint8Array.from(atob(parsed.body_base64), (c) =>
					c.charCodeAt(0),
				);
				parsed.body = new TextDecoder().decode(bytes);
			} catch {
				parsed.body = "";
			}
		}
		return parsed;
	}

	httpGet(
		url: string,
		headers: Record<string, string> = {},
	): { status: number; headers: Record<string, string>; body: string } | null {
		return this.httpRequest(0, url, headers);
	}

	httpPost(
		url: string,
		body: Uint8Array | null = null,
		headers: Record<string, string> = {},
	): { status: number; headers: Record<string, string>; body: string } | null {
		return this.httpRequest(1, url, headers, body);
	}

	// Finalize
	success(): ExecutionResult {
		this._result.exec("exec_out");
		return this._result;
	}

	fail(error: string): ExecutionResult {
		this._result.error = error;
		return this._result;
	}

	finish(): ExecutionResult {
		return this._result;
	}
}
