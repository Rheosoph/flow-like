export const BOT_HOST_FRESH_MS = 3 * 60_000;

export type BotHostState = "ready" | "offline" | "never";

/** Whether a device's agent polled for bots recently, polled once, or never did (older hubs and agents omit it). */
export function botHostState(
	seenAt: number | null | undefined,
	now: number = Date.now(),
): BotHostState {
	if (seenAt === null || seenAt === undefined) return "never";
	return now - seenAt <= BOT_HOST_FRESH_MS ? "ready" : "offline";
}
