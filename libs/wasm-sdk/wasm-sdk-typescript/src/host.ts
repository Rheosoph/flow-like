export type FlowPath = Record<string, string | null> & {
	path: string;
	store_ref: string;
	cache_store_ref: string | null;
};

export interface HostBridge {
	log(level: number, message: string): void;
	stream(eventType: string, data: string): void;
	streamText(content: string): void;

	getVariable(name: string): unknown;
	setVariable(name: string, value: unknown): boolean;
	deleteVariable(name: string): void;
	hasVariable(name: string): boolean;

	cacheGet(key: string): unknown;
	cacheSet(key: string, value: unknown): void;
	cacheDelete(key: string): void;
	cacheHas(key: string): boolean;

	timeNow(): number;
	random(): number;

	storageDir(nodeScoped: boolean): FlowPath | null;
	uploadDir(): FlowPath | null;
	cacheDir(nodeScoped: boolean, userScoped: boolean): FlowPath | null;
	userDir(nodeScoped: boolean): FlowPath | null;

	storageRead(flowPath: FlowPath): Uint8Array | null;
	storageWrite(flowPath: FlowPath, data: Uint8Array): boolean;
	storageList(flowPath: FlowPath): FlowPath[] | null;

	embedText(bit: Record<string, unknown>, texts: string[]): number[][] | null;

	getOAuthToken(provider: string): Record<string, unknown> | null;
	hasOAuthToken(provider: string): boolean;

	httpRequest(
		method: number,
		url: string,
		headers: string,
		body: Uint8Array | null,
	): string | null;
}

export class MockHostBridge implements HostBridge {
	logs: Array<[number, string]> = [];
	streams: Array<[string, string]> = [];
	variables: Record<string, unknown> = {};
	cacheData: Record<string, unknown> = {};
	storage: Record<string, Uint8Array> = {};
	oauthTokens: Record<string, Record<string, unknown>> = {};

	private _time = 0;
	private _randomValue = 42;
	private _embeddings: number[][] = [[0.1, 0.2, 0.3]];

	log(level: number, message: string): void {
		this.logs.push([level, message]);
	}

	stream(eventType: string, data: string): void {
		this.streams.push([eventType, data]);
	}

	streamText(content: string): void {
		this.streams.push(["text", content]);
	}

	getVariable(name: string): unknown {
		return this.variables[name] ?? null;
	}

	setVariable(name: string, value: unknown): boolean {
		this.variables[name] = value;
		return true;
	}

	deleteVariable(name: string): void {
		delete this.variables[name];
	}

	hasVariable(name: string): boolean {
		return name in this.variables;
	}

	cacheGet(key: string): unknown {
		return this.cacheData[key] ?? null;
	}

	cacheSet(key: string, value: unknown): void {
		this.cacheData[key] = value;
	}

	cacheDelete(key: string): void {
		delete this.cacheData[key];
	}

	cacheHas(key: string): boolean {
		return key in this.cacheData;
	}

	timeNow(): number {
		return this._time;
	}

	random(): number {
		return this._randomValue;
	}

	storageDir(nodeScoped: boolean): FlowPath {
		return {
			path: nodeScoped ? "storage/node" : "storage",
			store_ref: "mock_store",
			cache_store_ref: null,
		};
	}

	uploadDir(): FlowPath {
		return { path: "upload", store_ref: "mock_store", cache_store_ref: null };
	}

	cacheDir(_nodeScoped: boolean, _userScoped: boolean): FlowPath {
		return {
			path: "tmp/cache",
			store_ref: "mock_store",
			cache_store_ref: null,
		};
	}

	userDir(_nodeScoped: boolean): FlowPath {
		return {
			path: "users/mock",
			store_ref: "mock_store",
			cache_store_ref: null,
		};
	}

	storageRead(flowPath: FlowPath): Uint8Array | null {
		return this.storage[flowPath.path] ?? null;
	}

	storageWrite(flowPath: FlowPath, data: Uint8Array): boolean {
		this.storage[flowPath.path] = data;
		return true;
	}

	storageList(flowPath: FlowPath): FlowPath[] {
		const prefix = flowPath.path;
		return Object.keys(this.storage)
			.filter((k) => k.startsWith(prefix))
			.map((k) => ({
				path: k,
				store_ref: flowPath.store_ref,
				cache_store_ref: null,
			}));
	}

	embedText(_bit: Record<string, unknown>, texts: string[]): number[][] {
		return texts.map(() => [...this._embeddings[0]]);
	}

	getOAuthToken(provider: string): Record<string, unknown> | null {
		return this.oauthTokens[provider] ?? null;
	}

	hasOAuthToken(provider: string): boolean {
		return provider in this.oauthTokens;
	}

	httpRequest(
		_method: number,
		_url: string,
		_headers: string,
		_body: Uint8Array | null,
	): string | null {
		return JSON.stringify({ status: 200, headers: {}, body: "{}" });
	}
}

class DefaultHostBridge implements HostBridge {
	log(_level: number, _message: string): void {}
	stream(_eventType: string, _data: string): void {}
	streamText(_content: string): void {}
	getVariable(_name: string): unknown {
		return null;
	}
	setVariable(_name: string, _value: unknown): boolean {
		return false;
	}
	deleteVariable(_name: string): void {}
	hasVariable(_name: string): boolean {
		return false;
	}
	cacheGet(_key: string): unknown {
		return null;
	}
	cacheSet(_key: string, _value: unknown): void {}
	cacheDelete(_key: string): void {}
	cacheHas(_key: string): boolean {
		return false;
	}
	timeNow(): number {
		return 0;
	}
	random(): number {
		return 0;
	}
	storageDir(_nodeScoped: boolean): FlowPath | null {
		return null;
	}
	uploadDir(): FlowPath | null {
		return null;
	}
	cacheDir(_nodeScoped: boolean, _userScoped: boolean): FlowPath | null {
		return null;
	}
	userDir(_nodeScoped: boolean): FlowPath | null {
		return null;
	}
	storageRead(_flowPath: FlowPath): Uint8Array | null {
		return null;
	}
	storageWrite(_flowPath: FlowPath, _data: Uint8Array): boolean {
		return false;
	}
	storageList(_flowPath: FlowPath): FlowPath[] | null {
		return null;
	}
	embedText(
		_bit: Record<string, unknown>,
		_texts: string[],
	): number[][] | null {
		return null;
	}
	getOAuthToken(_provider: string): Record<string, unknown> | null {
		return null;
	}
	hasOAuthToken(_provider: string): boolean {
		return false;
	}
	httpRequest(
		_method: number,
		_url: string,
		_headers: string,
		_body: Uint8Array | null,
	): string | null {
		return null;
	}
}

let _host: HostBridge = new DefaultHostBridge();

export function setHost(host: HostBridge): void {
	_host = host;
}

export function getHost(): HostBridge {
	return _host;
}
