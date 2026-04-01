/**
 * WASM Component Model entry point for Flow-Like TypeScript nodes.
 *
 * Implements the WIT world exports (get-node, get-nodes, run, get-abi-version)
 * and bridges WIT host imports to the SDK's HostBridge.
 *
 * Build:
 *   npx esbuild src/app.ts --bundle --format=esm --outfile=dist/app.js --external:'flow-like:*'
 *   npx jco componentize dist/app.js --wit wit --world-name flow-like-node -o build/node.wasm
 */

import {
	getOauthToken as witGetOAuthToken,
	hasOauthToken as witHasOAuthToken,
} from "flow-like:node/auth@0.1.0";
import {
	cacheDelete as witCacheDelete,
	cacheGet as witCacheGet,
	cacheHas as witCacheHas,
	cacheSet as witCacheSet,
} from "flow-like:node/cache@0.1.0";
import { request as witHttpRequest } from "flow-like:node/http@0.1.0";
// WIT host imports — resolved by componentize-js from the WIT world
import { log } from "flow-like:node/logging@0.1.0";
import { random, timeNow } from "flow-like:node/metadata@0.1.0";
import { embedText as witEmbedText } from "flow-like:node/models@0.1.0";
import {
	cacheDir as witCacheDir,
	listFiles as witListFiles,
	readFile as witReadFile,
	storageDir as witStorageDir,
	uploadDir as witUploadDir,
	userDir as witUserDir,
	writeFile as witWriteFile,
} from "flow-like:node/storage@0.1.0";
import { emit, text as witStreamText } from "flow-like:node/streaming@0.1.0";
import {
	deleteVar,
	getVar,
	hasVar,
	setVar,
} from "flow-like:node/variables@0.1.0";

import {
	ABI_VERSION,
	Context,
	type FlowPath,
	type HostBridge,
	setHost,
} from "@flow-like/wasm-sdk-typescript";
import { getDefinition, run as runNode } from "./node";

class WitHostBridge implements HostBridge {
	log(level: number, message: string): void {
		log(level, message);
	}

	stream(eventType: string, data: string): void {
		emit(eventType, data);
	}

	streamText(content: string): void {
		witStreamText(content);
	}

	getVariable(name: string): unknown {
		const result = getVar(name);
		return result != null ? JSON.parse(result) : null;
	}

	setVariable(name: string, value: unknown): boolean {
		setVar(name, JSON.stringify(value));
		return true;
	}

	deleteVariable(name: string): void {
		deleteVar(name);
	}

	hasVariable(name: string): boolean {
		return hasVar(name);
	}

	cacheGet(key: string): unknown {
		const result = witCacheGet(key);
		return result != null ? JSON.parse(result) : null;
	}

	cacheSet(key: string, value: unknown): void {
		witCacheSet(key, JSON.stringify(value));
	}

	cacheDelete(key: string): void {
		witCacheDelete(key);
	}

	cacheHas(key: string): boolean {
		return witCacheHas(key);
	}

	timeNow(): number {
		return Number(timeNow());
	}

	random(): number {
		return random();
	}

	storageDir(nodeScoped: boolean): FlowPath | null {
		const result = witStorageDir(nodeScoped);
		return result != null ? JSON.parse(result) : null;
	}

	uploadDir(): FlowPath | null {
		const result = witUploadDir();
		return result != null ? JSON.parse(result) : null;
	}

	cacheDir(nodeScoped: boolean, userScoped: boolean): FlowPath | null {
		const result = witCacheDir(nodeScoped, userScoped);
		return result != null ? JSON.parse(result) : null;
	}

	userDir(nodeScoped: boolean): FlowPath | null {
		const result = witUserDir(nodeScoped);
		return result != null ? JSON.parse(result) : null;
	}

	storageRead(flowPath: FlowPath): Uint8Array | null {
		const result = witReadFile(JSON.stringify(flowPath));
		return result != null ? new Uint8Array(result) : null;
	}

	storageWrite(flowPath: FlowPath, data: Uint8Array): boolean {
		return witWriteFile(JSON.stringify(flowPath), Array.from(data));
	}

	storageList(flowPath: FlowPath): FlowPath[] | null {
		const result = witListFiles(JSON.stringify(flowPath));
		return result != null ? JSON.parse(result) : null;
	}

	embedText(bit: Record<string, unknown>, texts: string[]): number[][] | null {
		const result = witEmbedText(JSON.stringify(bit), JSON.stringify(texts));
		return result != null ? JSON.parse(result) : null;
	}

	getOAuthToken(provider: string): Record<string, unknown> | null {
		const result = witGetOAuthToken(provider);
		return result != null ? JSON.parse(result) : null;
	}

	hasOAuthToken(provider: string): boolean {
		return witHasOAuthToken(provider);
	}

	httpRequest(
		method: number,
		url: string,
		headers: string,
		body: Uint8Array | null,
	): string | null {
		return (
			witHttpRequest(
				method,
				url,
				headers,
				body != null ? Array.from(body) : undefined,
			) ?? null
		);
	}
}

setHost(new WitHostBridge());

export function getNode(): string {
	const def = getDefinition();
	return JSON.stringify([def.toDict()]);
}

export function getNodes(): string {
	return getNode();
}

export function run(inputJson: string): string {
	const ctx = Context.fromJSON(inputJson);
	const result = runNode(ctx);
	return JSON.stringify(result.toDict());
}

export function getAbiVersion(): number {
	return ABI_VERSION;
}
