import type { ExecutionRuntimeClient } from "./runtime-client";

const owners = new Map<string, (client: ExecutionRuntimeClient) => void>();

/** Preflight can finish before the provider's initial worker connection effect does. */
export function registerExecutionTransportOwner(
	taskId: string,
	attach: (client: ExecutionRuntimeClient) => void,
): () => void {
	owners.set(taskId, attach);
	return () => {
		if (owners.get(taskId) === attach) owners.delete(taskId);
	};
}

export function attachExecutionTransportOwner(
	taskId: string,
	client: ExecutionRuntimeClient,
): void {
	owners.get(taskId)?.(client);
}
