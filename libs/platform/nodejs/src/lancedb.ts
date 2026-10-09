import type { Connection } from "@lancedb/lancedb";
import type { FlowLikeClient } from "./index.js";

export type {
	Connection as LanceConnection,
	Table as LanceTable,
} from "@lancedb/lancedb";

/** Creates a connection with the full types from the installed LanceDB peer. */
export async function createLanceConnection(
	client: FlowLikeClient,
	appId: string,
	accessMode: "read" | "write" = "read",
	scope: "user" | "project" = "user",
): Promise<Connection> {
	return (await client.createLanceConnection(
		appId,
		accessMode,
		scope,
	)) as Connection;
}
