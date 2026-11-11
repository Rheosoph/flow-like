import type { IChannelClientDescriptor, IChannelPush } from "../schema/channel";
import { type ChannelPushOptions, errorMessage } from "./util";

export const AZURE_ACK_TIMEOUT_MS = 15_000;
export const AZURE_WEB_PUBSUB_SUBPROTOCOL = "json.webpubsub.azure.v1";
const WS_CONNECTING = 0;
const WS_OPEN = 1;

export type AzureWebPubSubChannelDescriptor = Extract<
	IChannelClientDescriptor,
	{ type: "azure_web_pubsub" }
>;

interface PendingAck {
	resolve: () => void;
	reject: (error: Error) => void;
}

interface AzureConnection {
	socket: WebSocket;
	open: Promise<void>;
	pending: Map<number, PendingAck>;
}

const connections = new Map<string, AzureConnection>();
let nextAckId = 1;

function ackError(payload: Record<string, unknown>): Error {
	const error = payload.error;
	if (error && typeof error === "object") {
		const record = error as Record<string, unknown>;
		const name = typeof record.name === "string" ? record.name : "Unknown";
		const message =
			typeof record.message === "string" && record.message
				? `: ${record.message}`
				: "";
		return new Error(
			`Azure Web PubSub rejected the message (${name})${message}`,
		);
	}
	return new Error("Azure Web PubSub rejected the message.");
}

function connect(
	channelId: string,
	descriptor: AzureWebPubSubChannelDescriptor,
): AzureConnection {
	if (typeof WebSocket === "undefined") {
		throw new Error("WebSocket is unavailable in this environment.");
	}
	const socket = new WebSocket(descriptor.url, AZURE_WEB_PUBSUB_SUBPROTOCOL);
	const pending = new Map<number, PendingAck>();
	const connection: AzureConnection = {
		socket,
		pending,
		open: new Promise<void>((resolve, reject) => {
			socket.addEventListener("open", () => resolve(), { once: true });
			socket.addEventListener(
				"error",
				() =>
					reject(
						new Error(
							`Azure Web PubSub connection for channel '${channelId}' failed.`,
						),
					),
				{ once: true },
			);
			socket.addEventListener(
				"close",
				(event) =>
					reject(
						new Error(
							`Azure Web PubSub connection for channel '${channelId}' closed (${event.code}${event.reason ? ` ${event.reason}` : ""}).`,
						),
					),
				{ once: true },
			);
		}),
	};
	connection.open.catch(() => undefined);

	socket.addEventListener("message", (event) => {
		let payload: unknown;
		try {
			payload = JSON.parse(String(event.data));
		} catch {
			return;
		}
		if (!payload || typeof payload !== "object") return;
		const record = payload as Record<string, unknown>;
		if (record.type !== "ack" || typeof record.ackId !== "number") return;
		const ack = pending.get(record.ackId);
		if (!ack) return;
		pending.delete(record.ackId);
		if (record.success === true) ack.resolve();
		else ack.reject(ackError(record));
	});
	socket.addEventListener("close", (event) => {
		const reason = new Error(
			`Azure Web PubSub connection for channel '${channelId}' closed before the message was acknowledged (${event.code}).`,
		);
		for (const ack of pending.values()) ack.reject(reason);
		pending.clear();
		if (connections.get(channelId) === connection) {
			connections.delete(channelId);
		}
	});

	connections.set(channelId, connection);
	return connection;
}

/**
 * One socket per channel, reused while it is connecting or open. A closed socket — including one
 * whose token expired — is replaced with a fresh connection from the handle passed in, which may
 * carry a refreshed token.
 */
function acquire(
	channelId: string,
	descriptor: AzureWebPubSubChannelDescriptor,
): AzureConnection {
	const existing = connections.get(channelId);
	if (
		existing &&
		(existing.socket.readyState === WS_CONNECTING ||
			existing.socket.readyState === WS_OPEN)
	) {
		return existing;
	}
	return connect(channelId, descriptor);
}

export async function pushAzureWebPubSub(
	descriptor: AzureWebPubSubChannelDescriptor,
	push: IChannelPush,
	options: ChannelPushOptions = {},
): Promise<void> {
	const connection = acquire(push.channel_id, descriptor);
	await connection.open;
	const ackId = nextAckId++;
	await new Promise<void>((resolve, reject) => {
		const settle = (outcome: () => void) => {
			clearTimeout(timer);
			options.signal?.removeEventListener("abort", onAbort);
			connection.pending.delete(ackId);
			outcome();
		};
		const onAbort = () =>
			settle(() =>
				reject(
					new Error(
						`Azure Web PubSub push for channel '${push.channel_id}' was aborted.`,
					),
				),
			);
		const timer = setTimeout(
			() =>
				settle(() =>
					reject(
						new Error(
							`Azure Web PubSub ack for channel '${push.channel_id}' timed out after ${AZURE_ACK_TIMEOUT_MS} ms.`,
						),
					),
				),
			AZURE_ACK_TIMEOUT_MS,
		);
		if (options.signal?.aborted) {
			onAbort();
			return;
		}
		options.signal?.addEventListener("abort", onAbort, { once: true });
		connection.pending.set(ackId, {
			resolve: () => settle(resolve),
			reject: (error) => settle(() => reject(error)),
		});
		try {
			connection.socket.send(
				JSON.stringify({
					type: "sendToGroup",
					group: descriptor.group,
					ackId,
					dataType: "json",
					data: push,
				}),
			);
		} catch (error) {
			settle(() =>
				reject(
					new Error(
						`Azure Web PubSub send for channel '${push.channel_id}' failed: ${errorMessage(error)}`,
					),
				),
			);
		}
	});
}

export function resetAzureWebPubSubConnections(): void {
	for (const connection of connections.values()) {
		try {
			connection.socket.close();
		} catch {
			// Closing is best-effort during a reset.
		}
	}
	connections.clear();
	nextAckId = 1;
}
