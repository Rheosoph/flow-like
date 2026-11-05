import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import type {
	IBotConnector,
	ICreateBotConnectorRequest,
	IUpdateBotConnectorRequest,
} from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import { invoke } from "@tauri-apps/api/core";

async function run<T>(
	operation: string,
	command: string,
	args: Record<string, unknown>,
): Promise<T> {
	try {
		return await invoke<T>(command, args);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		throw new Error(`Could not ${operation}: ${message}`);
	}
}

function expectConnector(value: unknown, operation: string): IBotConnector {
	if (!isRecord(value)) {
		throw new Error(`Could not ${operation}: this computer returned nothing`);
	}
	return value as unknown as IBotConnector;
}

/** The connectors of the bots that live on this computer. */
export const localConnectors = {
	async list(botId: string): Promise<IBotConnector[]> {
		return asArray(
			await run<IBotConnector[]>(
				`list the connectors of local bot ${botId}`,
				"local_bot_connectors",
				{ botId },
			),
		);
	},

	async create(
		botId: string,
		request: ICreateBotConnectorRequest,
	): Promise<IBotConnector> {
		const operation = `add a connector to local bot ${botId}`;
		return expectConnector(
			await run(operation, "local_bot_create_connector", { botId, request }),
			operation,
		);
	},

	async update(
		botId: string,
		connectorId: string,
		request: IUpdateBotConnectorRequest,
	): Promise<IBotConnector> {
		const operation = `change connector ${connectorId} of local bot ${botId}`;
		return expectConnector(
			await run(operation, "local_bot_update_connector", {
				botId,
				connectorId,
				request,
			}),
			operation,
		);
	},

	async refresh(botId: string, connectorId: string): Promise<IBotConnector> {
		const operation = `refresh connector ${connectorId} of local bot ${botId}`;
		return expectConnector(
			await run(operation, "local_bot_refresh_connector", {
				botId,
				connectorId,
			}),
			operation,
		);
	},

	async delete(botId: string, connectorId: string): Promise<void> {
		await run<void>(
			`remove connector ${connectorId} of local bot ${botId}`,
			"local_bot_delete_connector",
			{ botId, connectorId },
		);
	},
};
