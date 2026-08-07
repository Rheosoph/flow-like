import { mock } from "bun:test";

/**
 * Shared @tauri-apps/api/core mock for every test file in this package.
 * bun caches modules per process, so a module like ./index binds whichever
 * invoke mock was registered when it was first imported; a single shared
 * registry keeps all test files interoperable regardless of load order.
 */

export interface InvokeCall {
	cmd: string;
	args: Record<string, unknown> | undefined;
}

export const invokeCalls: InvokeCall[] = [];

/** Handler per command: a function receiving args, or a plain result value. */
export const invokeResults = new Map<string, unknown>();

mock.module("@tauri-apps/api/core", () => ({
	invoke: async (cmd: string, args?: Record<string, unknown>) => {
		invokeCalls.push({ cmd, args });
		const handler = invokeResults.get(cmd);
		if (typeof handler === "function") return handler(args);
		if (handler !== undefined) return handler;
		throw new Error(`Unhandled invoke: ${cmd}`);
	},
}));
