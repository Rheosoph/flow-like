import type { IChannelClientDescriptor, IChannelPush } from "../schema/channel";
import { sha256Hex, signAwsRequest } from "./aws-sigv4";
import {
	type ChannelPushOptions,
	errorMessage,
	readBodyExcerpt,
	timeoutSignal,
	unixSeconds,
} from "./util";

export const AWS_PUSH_TIMEOUT_MS = 30_000;
const IOT_DATA_SERVICE = "iotdata";
const DIRECT_MESSAGE_CONFIRMATION_TIMEOUT_S = 10;

export type AwsMqttChannelDescriptor = Extract<
	IChannelClientDescriptor,
	{ type: "aws_mqtt" }
>;

function endpointOrigin(endpoint: string): string {
	return /^https?:\/\//i.test(endpoint)
		? endpoint.replace(/\/+$/, "")
		: `https://${endpoint}`;
}

/** AWS IoT Core Direct Messaging: `SendDirectMessage` over HTTPS to the run's MQTT client. */
export function directMessageUrl(descriptor: AwsMqttChannelDescriptor): string {
	const clientId = encodeURIComponent(descriptor.target_client_id);
	const query = new URLSearchParams({
		topic: descriptor.topic,
		confirmation: "true",
		timeout: String(DIRECT_MESSAGE_CONFIRMATION_TIMEOUT_S),
	});
	return `${endpointOrigin(descriptor.endpoint)}/connections/${clientId}/messages?${query.toString()}`;
}

export async function pushAwsMqtt(
	descriptor: AwsMqttChannelDescriptor,
	push: IChannelPush,
	options: ChannelPushOptions = {},
): Promise<void> {
	const { credentials } = descriptor;
	if (credentials.expiration <= unixSeconds()) {
		throw new Error(
			`AWS IoT credentials for channel '${push.channel_id}' expired at ${credentials.expiration}; use the fallback transport.`,
		);
	}
	const url = directMessageUrl(descriptor);
	const body = JSON.stringify(push);
	const signed = await signAwsRequest(
		{
			method: "POST",
			url,
			headers: {
				"content-type": "application/json",
				"x-amz-content-sha256": await sha256Hex(body),
			},
			body,
			service: IOT_DATA_SERVICE,
			region: descriptor.region,
		},
		{
			accessKeyId: credentials.access_key_id,
			secretAccessKey: credentials.secret_access_key,
			sessionToken: credentials.session_token,
		},
	);

	const timeout = timeoutSignal(AWS_PUSH_TIMEOUT_MS, options.signal);
	try {
		let response: Response;
		try {
			response = await fetch(url, {
				method: "POST",
				headers: signed.headers,
				body,
				signal: timeout.signal,
			});
		} catch (error) {
			if (timeout.timedOut()) {
				throw new Error(
					`AWS IoT direct message for channel '${push.channel_id}' timed out after ${AWS_PUSH_TIMEOUT_MS} ms.`,
				);
			}
			if (timeout.signal.aborted) {
				throw new Error(
					`AWS IoT direct message for channel '${push.channel_id}' was aborted.`,
				);
			}
			throw new Error(
				`AWS IoT direct message for channel '${push.channel_id}' failed: ${errorMessage(error)}`,
			);
		}
		if (response.status === 404) {
			throw new Error(
				`The run behind channel '${push.channel_id}' is no longer listening (AWS IoT client '${descriptor.target_client_id}' is not connected).`,
			);
		}
		if (!response.ok) {
			const excerpt = await readBodyExcerpt(response);
			throw new Error(
				`AWS IoT direct message for channel '${push.channel_id}' failed (${response.status}): ${excerpt || response.statusText}`,
			);
		}
	} finally {
		timeout.dispose();
	}
}
