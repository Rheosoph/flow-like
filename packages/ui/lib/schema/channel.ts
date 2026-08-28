// TypeScript mirror of `packages/types/contracts/src/channel.rs` (snake_case wire JSON).

/** Temporary AWS credentials scoped to one channel. `expiration` is unix seconds. */
export interface IAwsTemporaryCredentials {
	access_key_id: string;
	secret_access_key: string;
	session_token: string;
	expiration: number;
}

/** How a client delivers an {@link IChannelPush} for a channel. */
export type IChannelClientDescriptor =
	| { type: "http"; push_url: string; token: string }
	| { type: "in_process" }
	| {
			type: "aws_mqtt";
			endpoint: string;
			region: string;
			target_client_id: string;
			topic: string;
			credentials: IAwsTemporaryCredentials;
	  }
	| {
			type: "azure_web_pubsub";
			url: string;
			group: string;
			/** Unix seconds after which the token no longer opens new connections. */
			expires_at: number;
	  }
	| {
			type: "gcp_firebase_rtdb";
			database_url: string;
			api_key: string;
			project_id: string;
			custom_token: string;
			inbox_path: string;
			inbound_path: string;
			expires_at: number;
	  };

export type IChannelTransport = IChannelClientDescriptor["type"];

/**
 * Everything a client needs to answer one request (or, with `request_id` unset, to push an
 * unsolicited message such as cancel/steer into the channel).
 */
export interface IChannelHandle {
	channel_id: string;
	request_id?: string | null;
	/** Unix seconds; the waiter stops listening after this. */
	expires_at: number;
	transport: IChannelClientDescriptor;
	/** Used when `transport` cannot be reached; always the API push endpoint. */
	fallback?: IChannelClientDescriptor | null;
}

export type IChannelPushKind = "reply" | "inbound" | "cancel";

/** The one message shape every transport carries from the client to the waiter. */
export interface IChannelPush {
	channel_id: string;
	request_id?: string | null;
	kind?: IChannelPushKind;
	value: unknown;
}
