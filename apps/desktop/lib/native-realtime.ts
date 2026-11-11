import type {
	RealtimePollRequest,
	RealtimePollResponse,
	RealtimeReply,
} from "@flow-like/flow-like-ui/lib/realtime/types";
import { Channel, invoke } from "@tauri-apps/api/core";

export type NativeRealtimeMessage =
	| {
			type: "state";
			state:
				| "connecting"
				| "connected"
				| "reconnecting"
				| "unauthorized"
				| "unsupported"
				| "signed_out"
				| "closed";
	  }
	| { type: "reply"; reply: RealtimeReply }
	| { type: "update"; delivery_id: string; response: RealtimePollResponse };

interface NativeRealtimeOptions {
	profileId: string;
	subject: string;
	issuer: string;
	request: RealtimePollRequest;
	transport?: "configured" | "http";
	onMessage: (message: NativeRealtimeMessage) => void | Promise<void>;
	onError?: (error: Error) => void;
}

/** Rust owns the account connection; this handle owns only the calling window's subscription. */
export function watchNativeRealtime(
	options: NativeRealtimeOptions,
): () => void {
	const subscriptionId = crypto.randomUUID();
	const events = new Channel<NativeRealtimeMessage>();
	let closed = false;
	let retry: ReturnType<typeof setTimeout> | undefined;
	let retryDelay = 1_000;
	const accepted = new Set<string>();
	const processing = new Set<string>();
	const acknowledge = (deliveryId: string) => {
		if (closed) return;
		void invoke("realtime_acknowledge", { subscriptionId, deliveryId }).catch(
			() => undefined,
		);
	};
	const state = (
		message: Extract<NativeRealtimeMessage, { type: "state" | "reply" }>,
	) => {
		if (closed) return;
		try {
			void Promise.resolve(options.onMessage(message)).catch(() => undefined);
		} catch {
			// State notifications do not acknowledge a replay batch.
		}
	};
	events.onmessage = (message) => {
		if (closed) return;
		if (message.type === "state" || message.type === "reply") {
			state(message);
			return;
		}
		const id = message.delivery_id;
		if (accepted.has(id)) {
			acknowledge(id);
			return;
		}
		// Rust retains one batch per subscription. Ignore a repeated delivery while
		// its callback is still running; a failed callback is retried on the next resend.
		if (processing.size > 0) return;
		processing.add(id);
		void Promise.resolve()
			.then(() => {
				if (!closed) return options.onMessage(message);
			})
			.then(() => {
				if (closed) return;
				accepted.add(id);
				const oldest = accepted.values().next().value;
				if (accepted.size > 64 && oldest) accepted.delete(oldest);
				acknowledge(id);
			})
			.catch(() => {
				// The native owner retains the unacknowledged batch for another attempt.
			})
			.finally(() => processing.delete(id));
	};
	const unsubscribe = () =>
		invoke("realtime_unsubscribe", { subscriptionId }).catch(() => undefined);
	const connect = async () => {
		try {
			await invoke("realtime_subscribe", {
				profileId: options.profileId,
				expectedSubject: options.subject,
				expectedIssuer: options.issuer,
				subscriptionId,
				request: options.request,
				transport: options.transport ?? "configured",
				events,
			});
			// A late registration must not outlive the effect that requested it.
			if (closed) await unsubscribe();
		} catch (error) {
			if (closed) return;
			const reason = String(error);
			if (
				/authentication session changed|subscriber navigated or signed out|Sign in to subscribe/.test(
					reason,
				)
			) {
				state({ type: "state", state: "signed_out" });
				return;
			}
			if (reason.startsWith("Invalid realtime")) {
				options.onError?.(new Error(reason));
				state({ type: "state", state: "closed" });
				return;
			}
			if (/command.*(?:not found|not allowed)|unknown command/i.test(reason)) {
				options.onError?.(new Error(reason));
				state({ type: "state", state: "unsupported" });
				return;
			}
			state({ type: "state", state: "reconnecting" });
			retry = setTimeout(() => void connect(), retryDelay);
			retryDelay = Math.min(retryDelay * 2, 30_000);
		}
	};
	void connect();
	return () => {
		if (closed) return;
		closed = true;
		clearTimeout(retry);
		events.onmessage = () => {};
		accepted.clear();
		processing.clear();
		void unsubscribe();
	};
}
