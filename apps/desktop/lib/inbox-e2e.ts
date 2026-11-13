import type { IInboxSealedEnvelope } from "@flow-like/flow-like-ui";
import type {
	IInboxCrypto,
	IInboxPublicKey,
} from "@flow-like/flow-like-ui/lib/inbox-e2e";
import { wasmFileCrypto } from "@flow-like/flow-like-ui/lib/inbox-e2e-browser";
import { invoke } from "@tauri-apps/api/core";

/**
 * The person's inbox key lives in this device's keychain; sealing and opening run natively.
 * Files carry their own keys, so they are sealed in the WebAssembly module like on the web.
 */
export const tauriInboxCrypto: IInboxCrypto = {
	key: (principalId) =>
		invoke<IInboxPublicKey>("inbox_e2e_key", { principalId }),
	seal: (scope, members, body) =>
		invoke<Record<string, IInboxSealedEnvelope>>("inbox_e2e_seal", {
			conversationId: scope.conversation_id,
			messageId: scope.message_id,
			senderId: scope.sender_id,
			members,
			body,
		}),
	sealSecret: (scope, keys, value) =>
		invoke<IInboxSealedEnvelope>("inbox_e2e_seal_secret", {
			botId: scope.bot_id,
			name: scope.name,
			keys,
			value,
		}),
	open: (principalId, entries) =>
		invoke<unknown[]>("inbox_e2e_open", { principalId, entries }),
	...wasmFileCrypto,
};
