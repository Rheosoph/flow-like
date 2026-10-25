import { describe, expect, test } from "bun:test";
import { IExecutionMode } from "../../lib/schema/flow/board";
import { deriveRunCapabilities } from "./flow-run-capabilities";

const online = { isOffline: false, hasRemoteExecute: true };

describe("deriveRunCapabilities", () => {
	test("a board with local-only nodes never runs on the server", () => {
		expect(
			deriveRunCapabilities({
				...online,
				executionMode: IExecutionMode.Hybrid,
				onlyOffline: true,
				canExecuteLocally: true,
			}),
		).toEqual({ canLocalExecute: true, canRemoteExecute: false });
	});

	test("a host whose local run goes to the server cannot run a local-only board", () => {
		expect(
			deriveRunCapabilities({
				...online,
				executionMode: IExecutionMode.Hybrid,
				onlyOffline: true,
				canExecuteLocally: false,
			}),
		).toEqual({ canLocalExecute: false, canRemoteExecute: false });
	});

	test("other boards keep both run paths on such a host", () => {
		expect(
			deriveRunCapabilities({
				...online,
				executionMode: IExecutionMode.Hybrid,
				onlyOffline: false,
				canExecuteLocally: false,
			}),
		).toEqual({ canLocalExecute: true, canRemoteExecute: true });
	});
});
