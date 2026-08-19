export const MAX_CONNECTION_MESSAGES = 10_000;
export const MESSAGE_BURST_CAPACITY = 120;
export const PUBLISH_BURST_CAPACITY = 60;
export const RATE_REFILL_WINDOW_MS = 10_000;

export class ConnectionRateLimiter {
	private messageTokens = MESSAGE_BURST_CAPACITY;
	private publishTokens = PUBLISH_BURST_CAPACITY;
	private messageCount = 0;
	private lastRefillAt: number;

	constructor(now = Date.now()) {
		this.lastRefillAt = now;
	}

	consume(publish: boolean, now = Date.now()): boolean {
		const elapsed = Math.max(0, now - this.lastRefillAt);
		this.lastRefillAt = Math.max(this.lastRefillAt, now);
		this.messageTokens = Math.min(
			MESSAGE_BURST_CAPACITY,
			this.messageTokens +
				(elapsed / RATE_REFILL_WINDOW_MS) * MESSAGE_BURST_CAPACITY,
		);
		this.publishTokens = Math.min(
			PUBLISH_BURST_CAPACITY,
			this.publishTokens +
				(elapsed / RATE_REFILL_WINDOW_MS) * PUBLISH_BURST_CAPACITY,
		);

		this.messageCount++;
		if (
			this.messageCount > MAX_CONNECTION_MESSAGES ||
			this.messageTokens < 1 ||
			(publish && this.publishTokens < 1)
		) {
			return false;
		}
		this.messageTokens -= 1;
		if (publish) this.publishTokens -= 1;
		return true;
	}
}
