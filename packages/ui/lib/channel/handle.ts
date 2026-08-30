import type { IChannelHandle } from "../schema/channel";

/** Structural check for a channel handle arriving on a stream (snake_case JSON from Rust). */
export function isChannelHandle(value: unknown): value is IChannelHandle {
	if (!value || typeof value !== "object" || Array.isArray(value)) return false;
	const record = value as Record<string, unknown>;
	const transport = record.transport;
	return (
		typeof record.channel_id === "string" &&
		record.channel_id.length > 0 &&
		typeof record.expires_at === "number" &&
		!!transport &&
		typeof transport === "object" &&
		typeof (transport as Record<string, unknown>).type === "string"
	);
}
