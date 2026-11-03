import type { IBotTurn, IBotTurnHost } from "@flow-like/flow-like-ui";
import { asArray } from "@flow-like/flow-like-ui/lib/response-shape";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const TURN_EVENT = "bot-host://turn";

/** Hands the host the way to end a turn of a bot that lives in the account. */
export type CancelAccountTurn = (turn: IBotTurn) => Promise<void>;

/** Turns the desktop's bot host works on, from `bot_host_turns` and `bot-host://turn`. */
export class BotTurnHost implements IBotTurnHost {
	constructor(
		private readonly isLocal: (botId: string) => Promise<boolean>,
		private readonly cancelAccountTurn: CancelAccountTurn,
	) {}

	async list(): Promise<IBotTurn[]> {
		return asArray(await invoke<IBotTurn[]>("bot_host_turns"));
	}

	subscribe(listener: (turn: IBotTurn) => void): Promise<() => void> {
		return listen<IBotTurn>(TURN_EVENT, (event) => listener(event.payload));
	}

	async cancel(turn: IBotTurn): Promise<void> {
		if (await this.isLocal(turn.bot_id)) {
			await invoke("bot_host_cancel_turn", { turnId: turn.id });
			return;
		}
		await this.cancelAccountTurn(turn);
	}
}
