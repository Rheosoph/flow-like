import type { IInboxSealedEnvelope } from "@flow-like/flow-like-ui";
import type {
	IInboxCrypto,
	IInboxPublicKey,
} from "@flow-like/flow-like-ui/lib/inbox-e2e";
import { invoke } from "@tauri-apps/api/core";

/** The person's inbox key lives in this device's keychain; sealing and opening run natively. */
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
	open: (principalId, entries) =>
		invoke<unknown[]>("inbox_e2e_open", { principalId, entries }),
};
