import type { IChannelPush } from "../schema/channel";

// Keep these limits and the wire fields in sync with channels/src/aws/reply_chunks.rs.
// AWS IoT applies its 128 KiB limit to the entire serialized MQTT payload.
export const AWS_MQTT_PAYLOAD_BYTES = 128 * 1024;
export const AWS_REPLY_CHUNK_BYTES = 64 * 1024;
export const AWS_MAX_REPLY_BYTES = 2 * 1024 * 1024;

function base64(bytes: Uint8Array): string {
	let binary = "";
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return btoa(binary);
}

/** Preserve the complete reply, including its envelope, across bounded AWS messages. */
export function awsReplyPayloads(push: IChannelPush): string[] {
	const body = JSON.stringify(push);
	const bytes = new TextEncoder().encode(body);
	if (bytes.length <= AWS_MQTT_PAYLOAD_BYTES) return [body];
	if ((push.kind ?? "reply") !== "reply" || !push.request_id) {
		throw new Error(
			"AWS channel messages larger than 128 KiB must be replies with a request_id.",
		);
	}
	if (bytes.length > AWS_MAX_REPLY_BYTES) {
		throw new Error(
			"AWS channel reply exceeds the 2 MiB reassembly limit; request a smaller result.",
		);
	}

	const transferId = crypto.randomUUID();
	const total = Math.ceil(bytes.length / AWS_REPLY_CHUNK_BYTES);
	const payloads: string[] = [];
	for (let index = 0; index < total; index++) {
		const payload = JSON.stringify({
			channel_id: push.channel_id,
			request_id: push.request_id,
			kind: "reply_chunk",
			transfer_id: transferId,
			index,
			total,
			total_bytes: bytes.length,
			data: base64(
				bytes.subarray(
					index * AWS_REPLY_CHUNK_BYTES,
					(index + 1) * AWS_REPLY_CHUNK_BYTES,
				),
			),
		});
		if (new TextEncoder().encode(payload).length > AWS_MQTT_PAYLOAD_BYTES) {
			throw new Error(
				"AWS channel reply identifiers leave insufficient space for the chunk envelope.",
			);
		}
		payloads.push(payload);
	}
	return payloads;
}
