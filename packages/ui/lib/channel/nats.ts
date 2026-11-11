import type { NatsConnection, Subscription } from "@nats-io/nats-core";
import type { IChannelClientDescriptor, IChannelPush } from "../schema/channel";
import { type ChannelPushOptions, timeoutSignal, unixSeconds } from "./util";

export const NATS_ACK_TIMEOUT_MS = 5_000;
const CONNECT_TIMEOUT_MS = 5_000;
const MAX_CONNECTIONS = 32;
const MAX_PENDING_PER_CONNECTION = 32;
const IDLE_TIMEOUT_MS = 60_000;
const MAX_PAYLOAD_BYTES = 1024 * 1024;
const ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;

export type NatsChannelDescriptor = Extract<
	IChannelClientDescriptor,
	{ type: "nats" }
>;

interface Connection {
	descriptor: NatsChannelDescriptor;
	ready: Promise<NatsConnection>;
	closed: boolean;
	users: number;
	pending: Set<(error: Error) => void>;
	expiry: ReturnType<typeof setTimeout>;
	idle?: ReturnType<typeof setTimeout>;
}

const connections = new Map<string, Connection>();

class RejectedPush extends Error {}

function validate(descriptor: NatsChannelDescriptor, channelId: string): void {
	if (
		!ID_PATTERN.test(channelId) ||
		descriptor.subject !== `channel.${channelId}.inbox`
	) {
		throw new Error("The NATS grant does not belong to this channel.");
	}
	const url = new URL(descriptor.url);
	if (
		!["ws:", "wss:"].includes(url.protocol) ||
		url.username ||
		url.password ||
		url.search ||
		url.hash
	) {
		throw new Error(
			"The NATS endpoint must be a WebSocket URL without credentials or query parameters.",
		);
	}
	if (
		!Number.isFinite(descriptor.expires_at) ||
		descriptor.expires_at <= unixSeconds()
	) {
		throw new Error("The NATS channel grant has expired.");
	}
	if (!descriptor.token)
		throw new Error("The NATS channel grant has no token.");
}

function close(channelId: string, connection: Connection): void {
	if (connection.closed) return;
	connection.closed = true;
	for (const reject of connection.pending)
		reject(new Error("The NATS connection closed before acknowledgement."));
	connection.pending.clear();
	clearTimeout(connection.expiry);
	clearTimeout(connection.idle);
	if (connections.get(channelId) === connection) connections.delete(channelId);
	void connection.ready.then((client) => client.close()).catch(() => {});
}

function acquire(
	channelId: string,
	descriptor: NatsChannelDescriptor,
): Connection {
	const existing = connections.get(channelId);
	if (existing) {
		if (
			!existing.closed &&
			existing.descriptor.token === descriptor.token &&
			existing.descriptor.url === descriptor.url &&
			existing.descriptor.subject === descriptor.subject
		) {
			clearTimeout(existing.idle);
			return existing;
		}
		close(channelId, existing);
	}
	if (connections.size >= MAX_CONNECTIONS) {
		const idle = [...connections].find(
			([, connection]) => connection.users === 0,
		);
		if (!idle)
			throw new Error("Too many NATS channels are delivering messages.");
		close(...idle);
	}
	const connection: Connection = {
		descriptor: { ...descriptor },
		closed: false,
		users: 0,
		pending: new Set(),
		expiry: setTimeout(
			() => close(channelId, connection),
			Math.min(2_147_483_647, descriptor.expires_at * 1000 - Date.now()),
		),
		ready: import("@nats-io/nats-core").then(
			({ wsconnect, jwtAuthenticator }) =>
				wsconnect({
					servers: descriptor.url,
					authenticator: jwtAuthenticator(descriptor.token),
					name: `flow-like-channel-${channelId}`,
					timeout: CONNECT_TIMEOUT_MS,
					reconnect: false,
					ignoreClusterUpdates: true,
				}),
		),
	};
	connections.set(channelId, connection);
	void connection.ready
		.then((client) => {
			if (connection.closed) return client.close();
			void client.closed().then(() => close(channelId, connection));
		})
		.catch(() => close(channelId, connection));
	return connection;
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		const abort = () =>
			reject(new Error("NATS channel delivery was aborted or timed out."));
		if (signal.aborted) return abort();
		signal.addEventListener("abort", abort, { once: true });
		promise
			.then(resolve, reject)
			.finally(() => signal.removeEventListener("abort", abort));
	});
}

async function send(
	connection: Connection,
	client: NatsConnection,
	descriptor: NatsChannelDescriptor,
	id: string,
	payload: Uint8Array,
	signal: AbortSignal,
): Promise<void> {
	let subscription: Subscription | undefined;
	let subscriptionError: Error | undefined;
	let rejectAck: (error: Error) => void = () => {};
	const reply = `${descriptor.subject}.ack.${id}`;
	const timeout = timeoutSignal(NATS_ACK_TIMEOUT_MS, signal);
	const ack = new Promise<void>((resolve, reject) => {
		rejectAck = reject;
		connection.pending.add(reject);
		subscription = client.subscribe(reply, {
			max: 1,
			callback(error, message) {
				if (error) {
					subscriptionError = new Error(
						"The NATS acknowledgement subscription failed.",
					);
					return reject(subscriptionError);
				}
				if (message.headers?.code === 503)
					return reject(new Error("The channel executor is not listening."));
				try {
					const value = message.json<{ accepted?: boolean; error?: string }>();
					if (value?.accepted === true) resolve();
					else
						reject(
							new RejectedPush(
								value?.error || "The channel executor rejected the message.",
							),
						);
				} catch {
					reject(
						new RejectedPush(
							"The channel executor returned an invalid acknowledgement.",
						),
					);
				}
			},
		});
	});
	// Errors can arrive while the subscription is being flushed.
	void ack.catch(() => {});
	try {
		await abortable(client.flush(), timeout.signal);
		if (subscriptionError) throw subscriptionError;
		if (timeout.signal.aborted)
			throw new Error("NATS channel delivery was aborted.");
		client.publish(descriptor.subject, payload, { reply });
		await abortable(ack, timeout.signal);
	} finally {
		timeout.dispose();
		connection.pending.delete(rejectAck);
		subscription?.unsubscribe();
	}
}

/** Resolves only after the executor accepts the push, using the same ID for every attempt. */
export async function pushNats(
	descriptor: NatsChannelDescriptor,
	push: IChannelPush,
	options: ChannelPushOptions = {},
): Promise<void> {
	validate(descriptor, push.channel_id);
	if (options.signal?.aborted)
		throw new Error("NATS channel delivery was aborted.");
	const id = options.deliveryId ?? crypto.randomUUID();
	if (!ID_PATTERN.test(id)) throw new Error("Invalid NATS delivery ID.");
	const payload = new TextEncoder().encode(JSON.stringify({ id, push }));
	if (payload.byteLength > MAX_PAYLOAD_BYTES)
		throw new Error("The NATS channel message exceeds 1 MiB.");
	for (let attempt = 0; attempt < 2; attempt++) {
		validate(descriptor, push.channel_id);
		const connection = acquire(push.channel_id, descriptor);
		if (connection.users >= MAX_PENDING_PER_CONNECTION)
			throw new Error("Too many pending NATS messages for this channel.");
		connection.users++;
		const timeout = timeoutSignal(
			CONNECT_TIMEOUT_MS + NATS_ACK_TIMEOUT_MS,
			options.signal,
		);
		try {
			const client = await abortable(connection.ready, timeout.signal);
			await send(connection, client, descriptor, id, payload, timeout.signal);
			return;
		} catch (error) {
			if (
				options.signal?.aborted ||
				error instanceof RejectedPush ||
				attempt === 1
			)
				throw error;
			close(push.channel_id, connection);
		} finally {
			timeout.dispose();
			connection.users--;
			if (!connection.closed && connection.users === 0) {
				connection.idle = setTimeout(
					() => close(push.channel_id, connection),
					IDLE_TIMEOUT_MS,
				);
			}
		}
	}
}

export function resetNatsConnections(): void {
	for (const [channelId, connection] of connections)
		close(channelId, connection);
}
