import type {
	IBotComputerHost,
	IBotComputerView,
} from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";

/** The browser and screen of the bots that run on this desktop. */
export class BotComputerHost implements IBotComputerHost {
	view(botId: string): Promise<IBotComputerView> {
		return invoke<IBotComputerView>("bot_computer_view", { botId });
	}

	stop(botId: string): Promise<void> {
		return invoke<void>("bot_computer_stop", { botId });
	}
}
