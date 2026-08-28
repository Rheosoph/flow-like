import type { IChannelPush } from "../schema/channel";

/** Result of the desktop `channel_push` command; anything but `delivered` is a failed push. */
export type InProcessPushResult =
	| "delivered"
	| "unknown_channel"
	| "unknown_request"
	| "expired"
	| "duplicate"
	| "full";

export async function pushInProcess(push: IChannelPush): Promise<void> {
	const { invoke } = await import("@tauri-apps/api/core");
	const result = await invoke<InProcessPushResult | string>("channel_push", {
		push,
	});
	if (result !== "delivered") {
		const target = push.request_id
			? `request '${push.request_id}' on channel '${push.channel_id}'`
			: `channel '${push.channel_id}'`;
		throw new Error(
			`In-process push to ${target} was not delivered: ${result}`,
		);
	}
}
