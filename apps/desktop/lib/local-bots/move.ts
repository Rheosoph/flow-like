import type {
	IBot,
	IBotMoveIssue,
	IBotMoveProgress,
	IBotMoveReport,
	ILocalBotHost,
	IMovedBot,
} from "@flow-like/flow-like-ui";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
	type IAccountSession,
	type ILocalBotsBackend,
	accountSession,
	hubAccessToken,
} from "./account";
import { errorCode, localBots } from "./commands";
import { localRegistries } from "./registry";

const MOVE_EVENT = "local-bots://move";

export const BOT_SECRET_UNREADABLE = "BOT_SECRET_UNREADABLE";

/** A refused or interrupted move. `code` is the server's or the store's; repeating the move resumes it. */
export class BotMoveError extends Error {
	readonly code?: string;
	readonly issue?: IBotMoveIssue;
	/** The step that stopped (`check`, `import_bot`, `conversations`, …). */
	readonly step?: string;

	constructor(
		message: string,
		options: {
			code?: string;
			issue?: IBotMoveIssue;
			step?: string;
			cause?: unknown;
		} = {},
	) {
		super(message, { cause: options.cause });
		this.name = "BotMoveError";
		this.code = options.code ?? options.issue?.code;
		this.issue = options.issue;
		this.step = options.step;
	}
}

/** A chat whose id on this computer now stands for its id in the account. */
interface IMovedConversation {
	local_id: string;
	account_id: string;
}

function movedConversations(value: unknown): IMovedConversation[] {
	if (!Array.isArray(value)) return [];
	return value.filter(
		(item): item is IMovedConversation =>
			isRecord(item) &&
			typeof item.local_id === "string" &&
			typeof item.account_id === "string",
	);
}

function textOf(value: unknown): string | undefined {
	return typeof value === "string" && value.trim() ? value : undefined;
}

/** The command's `{ code, message, step, issue? }` as a {@link BotMoveError}. */
function moveError(error: unknown, fallback: string): BotMoveError {
	if (!isRecord(error)) {
		return new BotMoveError(`${fallback}: ${textOf(error) ?? String(error)}`, {
			cause: error,
		});
	}
	return new BotMoveError(textOf(error.message) ?? fallback, {
		code: errorCode(error),
		issue: isRecord(error.issue)
			? (error.issue as unknown as IBotMoveIssue)
			: undefined,
		step: textOf(error.step),
		cause: error,
	});
}

interface ISignedIn {
	readonly session: IAccountSession;
	readonly accessToken: string;
}

/**
 * Moves bots from this computer to the signed-in account. Rust runs the review and the move
 * (`local_bots_check_move`, `local_bots_move`, one move at a time) against the active profile's
 * hub; this passes the sign-in and keeps local ids pointing at the account's.
 */
export class LocalBotMover implements ILocalBotHost {
	constructor(private readonly backend: ILocalBotsBackend) {}

	list(): Promise<IBot[]> {
		return localBots.list();
	}

	private signedIn(botId: string): ISignedIn {
		const session = accountSession(this.backend);
		const accessToken = session ? hubAccessToken(session) : undefined;
		if (!session || !accessToken) {
			throw new BotMoveError(`Sign in to move bot ${botId} to your account`);
		}
		return { session, accessToken };
	}

	async checkMove(botId: string): Promise<IBotMoveReport> {
		const { accessToken } = this.signedIn(botId);
		const fallback = `Could not check bot ${botId} against your account`;
		let report: unknown;
		try {
			report = await invoke("local_bots_check_move", { botId, accessToken });
		} catch (error) {
			throw moveError(error, fallback);
		}
		if (!isRecord(report) || !Array.isArray(report.issues)) {
			throw new BotMoveError(`${fallback}: this computer returned no review`);
		}
		return report as unknown as IBotMoveReport;
	}

	onProgress(
		listener: (progress: IBotMoveProgress) => void,
	): Promise<() => void> {
		return listen<IBotMoveProgress>(MOVE_EVENT, (event) =>
			listener(event.payload),
		);
	}

	/** Afterwards the local ids of the bot and its moved chats stand for their account ids. */
	async move(botId: string): Promise<IMovedBot> {
		const { session, accessToken } = this.signedIn(botId);
		const fallback = `Could not move bot ${botId} to your account`;
		const registries = localRegistries(this.backend);
		const follow = (moved: readonly IMovedConversation[]) => {
			for (const { local_id, account_id } of moved) {
				registries.conversations.moveToAccount(
					session.sub,
					local_id,
					account_id,
				);
			}
		};
		let moved: unknown;
		try {
			moved = await invoke("local_bots_move", { botId, accessToken });
		} catch (error) {
			if (isRecord(error)) follow(movedConversations(error.conversations));
			throw moveError(error, fallback);
		}
		const bot = isRecord(moved) ? moved.bot : undefined;
		if (!isRecord(bot) || typeof bot.id !== "string" || !bot.id) {
			throw new BotMoveError(`${fallback}: this computer returned no bot`);
		}
		follow(movedConversations(isRecord(moved) ? moved.conversations : []));
		registries.bots.moveToAccount(session.sub, botId, bot.id);
		const warnings = isRecord(moved) ? moved.warnings : undefined;
		return {
			bot: bot as unknown as IBot,
			warnings: typeof warnings === "number" ? warnings : 0,
		};
	}
}
