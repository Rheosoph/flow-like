import type {
	IBotComputerHost,
	IBotComputerView,
	IBotInputEvent,
} from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";

/**
 * The browser and screen of the bots that run on this desktop, and of device bots over the
 * encrypted device tunnel.
 */
export class BotComputerHost implements IBotComputerHost {
	view(botId: string, deviceId?: string | null): Promise<IBotComputerView> {
		return deviceId
			? invoke<IBotComputerView>("device_bot_view", { deviceId, botId })
			: invoke<IBotComputerView>("bot_computer_view", { botId });
	}

	stop(botId: string, deviceId?: string | null): Promise<void> {
		return deviceId
			? invoke<void>("device_bot_stop", { deviceId, botId })
			: invoke<void>("bot_computer_stop", { botId });
	}

	takeOver(botId: string, deviceId?: string | null): Promise<void> {
		return deviceId
			? invoke<void>("device_bot_take_over", { deviceId, botId })
			: invoke<void>("bot_computer_take_over", { botId });
	}

	handBack(botId: string, deviceId?: string | null): Promise<void> {
		return deviceId
			? invoke<void>("device_bot_hand_back", { deviceId, botId })
			: invoke<void>("bot_computer_hand_back", { botId });
	}

	input(
		botId: string,
		event: IBotInputEvent,
		deviceId?: string | null,
	): Promise<void> {
		if (!deviceId)
			return Promise.reject(
				new Error(
					"A bot on this computer is driven with your own mouse and keyboard",
				),
			);
		return invoke<void>("device_bot_input", { deviceId, botId, event });
	}
}
