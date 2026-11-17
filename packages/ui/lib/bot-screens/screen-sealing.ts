import type { IInboxSealedEnvelope } from "../../state/backend-state/inbox-state";
import type { IInboxCrypto } from "../inbox-e2e";
import {
	type ScreenTrustStore,
	type TrustedHost,
	createScreenTrustStore,
} from "./screen-trust";

export interface ScreenSealScope {
	session: string;
	message: "offer" | "answer";
}

export interface ScreenPinnedKey {
	key_id: string;
	public_key: string;
}

/**
 * Seals the SDP of a connection to one pinned key of the account's own computers and opens what
 * they sealed. The key directory is never read: trust comes from QR pairing alone.
 */
export interface ScreenSealing {
	/** This device's own key; its id goes into the offer so the host finds the pin. */
	own(): Promise<ScreenPinnedKey>;
	seal(
		scope: ScreenSealScope,
		body: unknown,
		recipient: ScreenPinnedKey,
	): Promise<IInboxSealedEnvelope>;
	/** The sealed body; throws `ScreenNoKeyError` when none of this device's keys opens it. */
	open(
		scope: ScreenSealScope,
		envelope: IInboxSealedEnvelope,
	): Promise<unknown>;
	trusted(): Promise<TrustedHost[]>;
	pin(host: TrustedHost): Promise<void>;
	unpin(keyId: string): Promise<void>;
}

export class ScreenNoKeyError extends Error {
	readonly code = "no_key";

	constructor(message = "This device's key does not open the message") {
		super(message);
		this.name = "ScreenNoKeyError";
	}
}

export interface ScreenSealingDeps {
	crypto: IInboxCrypto;
	sub: () => string | undefined;
	trust?: ScreenTrustStore;
}

const conversationOf = (scope: ScreenSealScope) =>
	`bot-screen:${scope.session}`;

/** The sealing port over the app's inbox crypto and this device's pinned computers. */
export function createScreenSealing(deps: ScreenSealingDeps): ScreenSealing {
	const trust = deps.trust ?? createScreenTrustStore();

	const requireSub = (): string => {
		const sub = deps.sub();
		if (!sub)
			throw new Error("Live bot screens need an account; sign in first");
		return sub;
	};

	return {
		async own() {
			const { key_id, public_key } = await deps.crypto.key(requireSub());
			return { key_id, public_key };
		},
		async seal(scope, body, recipient) {
			const sub = requireSub();
			const envelopes = await deps.crypto.seal(
				{
					conversation_id: conversationOf(scope),
					message_id: scope.message,
					sender_id: sub,
				},
				[
					{
						principal_id: sub,
						keys: [
							{
								key_id: recipient.key_id,
								public_key: recipient.public_key,
							},
						],
					},
				],
				body,
			);
			const envelope = envelopes[sub];
			if (!envelope)
				throw new Error("Sealing the connection offer produced no envelope");
			return envelope;
		},
		async open(scope, envelope) {
			const sub = requireSub();
			const [body] = await deps.crypto.open(sub, [
				{
					conversation_id: conversationOf(scope),
					message_id: scope.message,
					sender_id: sub,
					envelope,
				},
			]);
			if (body === null || body === undefined) throw new ScreenNoKeyError();
			return body;
		},
		trusted() {
			const sub = deps.sub();
			return sub ? trust.list(sub) : Promise.resolve([]);
		},
		pin(host) {
			return trust.put(requireSub(), host);
		},
		unpin(keyId) {
			return trust.remove(requireSub(), keyId);
		},
	};
}
