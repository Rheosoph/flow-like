import type {
	IBotEncryptionHost,
	IBotEncryptionStatus,
} from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";

/** The inbox keys of the encryption bots this desktop runs, kept in its keychain. */
export class BotEncryptionHost implements IBotEncryptionHost {
	status(botId: string): Promise<IBotEncryptionStatus> {
		return invoke<IBotEncryptionStatus>("bot_host_e2e_status", { botId });
	}

	rotate(botId: string): Promise<IBotEncryptionStatus> {
		return invoke<IBotEncryptionStatus>("bot_host_e2e_rotate_key", { botId });
	}
}
