import { jwtAuthenticator, wsconnect } from "@nats-io/nats-core";
import { resetNatsConnections } from "../../packages/ui/lib/channel/nats";
import { rawPushChannel } from "../../packages/ui/lib/channel/transport";
import type { IChannelHandle } from "../../packages/ui/lib/schema/channel";

export const channels = {
	push: rawPushChannel,
	reset: resetNatsConnections,
	async connectionRejected(handle: IChannelHandle): Promise<boolean> {
		if (handle.transport.type !== "nats")
			throw new Error("NATS handle required");
		try {
			const client = await wsconnect({
				servers: handle.transport.url,
				authenticator: jwtAuthenticator(handle.transport.token),
				reconnect: false,
				timeout: 2_000,
			});
			await client.close();
			return false;
		} catch {
			return true;
		}
	},
	async permissionRejected(
		handle: IChannelHandle,
		action: "publish" | "subscribe",
		subject: string,
	): Promise<boolean> {
		if (handle.transport.type !== "nats")
			throw new Error("NATS handle required");
		const client = await wsconnect({
			servers: handle.transport.url,
			authenticator: jwtAuthenticator(handle.transport.token),
			reconnect: false,
			timeout: 2_000,
		});
		let timer: ReturnType<typeof setTimeout> | undefined;
		try {
			const rejected = (async () => {
				for await (const status of client.status()) {
					if (
						status.type === "error" &&
						/permission/i.test(status.error.message)
					)
						return true;
				}
				return false;
			})();
			if (action === "publish")
				client.publish(subject, new TextEncoder().encode("forbidden"));
			else client.subscribe(subject, { callback: () => {} });
			return await Promise.race([
				rejected,
				new Promise<boolean>((resolve) => {
					timer = setTimeout(() => resolve(false), 2_000);
				}),
			]);
		} finally {
			clearTimeout(timer);
			await client.close();
		}
	},
};

declare global {
	interface Window {
		natsChannels: typeof channels;
	}
}
window.natsChannels = channels;
