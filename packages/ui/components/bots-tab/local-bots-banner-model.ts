import type { IBot } from "../../state/backend-state/bot-state";

export const BANNER_HIDDEN_KEY = "flow-like:bots:local-banner-hidden";

/** Who hid the banner and when. A bot made on this computer after that brings it back. */
export interface ILocalBotsBannerHidden {
	readonly account: string;
	readonly at: number;
}

export function parseBannerHidden(
	raw: string | null,
): ILocalBotsBannerHidden | null {
	if (!raw) return null;
	try {
		const value: unknown = JSON.parse(raw);
		if (typeof value !== "object" || value === null) return null;
		const { account, at } = value as Record<string, unknown>;
		return typeof account === "string" &&
			typeof at === "number" &&
			Number.isFinite(at)
			? { account, at }
			: null;
	} catch {
		return null;
	}
}

export function isBannerHidden(
	hidden: ILocalBotsBannerHidden | null,
	account: string,
	bots: readonly Pick<IBot, "created_at">[],
): boolean {
	if (hidden?.account !== account) return false;
	return !bots.some((bot) => Date.parse(bot.created_at) > hidden.at);
}

export function readBannerHidden(): ILocalBotsBannerHidden | null {
	try {
		return parseBannerHidden(localStorage.getItem(BANNER_HIDDEN_KEY));
	} catch {
		return null;
	}
}

export function writeBannerHidden(hidden: ILocalBotsBannerHidden): void {
	try {
		localStorage.setItem(BANNER_HIDDEN_KEY, JSON.stringify(hidden));
	} catch {
		// Without browser storage the banner stays hidden until the next visit.
	}
}
