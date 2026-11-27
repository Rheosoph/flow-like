import type {
	IBotDiscordInput,
	IBotDiscordState,
	IBotDiscordView,
} from "@flow-like/flow-like-ui/state/backend-state/bot-channels-state";
import { invoke } from "@tauri-apps/api/core";
import type { TauriBackend } from "../tauri-provider";

export class BotDiscordState implements IBotDiscordState {
	constructor(private readonly backend: TauriBackend) {}

	get(botId: string): Promise<IBotDiscordView | null> {
		return invoke<IBotDiscordView | null>("bot_discord_get", { botId });
	}

	set(botId: string, input: IBotDiscordInput): Promise<IBotDiscordView> {
		return invoke<IBotDiscordView>("bot_discord_set", { botId, input });
	}

	remove(botId: string): Promise<void> {
		return invoke<void>("bot_discord_remove", { botId });
	}
}
