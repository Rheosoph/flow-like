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

const MIB = 1024 * 1024;

export type FrameBudgetLimits = {
	frames: number;
	framesPerSecond: number;
	bytes: number;
	bytesPerSecond: number;
};

/** What a device may send to one controller participant. */
export const DEVICE_PARTICIPANT_BUDGET: FrameBudgetLimits = {
	frames: 256,
	framesPerSecond: 64,
	bytes: 4 * MIB,
	bytesPerSecond: MIB,
};
/** What a device may send across every participant it addresses. */
export const DEVICE_AGGREGATE_BUDGET: FrameBudgetLimits = {
	frames: 1024,
	framesPerSecond: 256,
	bytes: 16 * MIB,
	bytesPerSecond: 4 * MIB,
};
/** What one account may send to one device across all of its sockets. */
export const CONTROLLER_BUDGET: FrameBudgetLimits = {
	frames: 256,
	framesPerSecond: 64,
	bytes: 4 * MIB,
	bytesPerSecond: MIB,
};

class TokenBucket {
	private tokens: number;
	private updatedAt: number;

	constructor(
		private readonly capacity: number,
		private readonly refillPerSecond: number,
		now: number,
	) {
		this.tokens = capacity;
		this.updatedAt = now;
	}

	private refill(now: number) {
		const elapsed = Math.max(0, now - this.updatedAt);
		this.updatedAt = Math.max(this.updatedAt, now);
		this.tokens = Math.min(
			this.capacity,
			this.tokens + (elapsed / 1000) * this.refillPerSecond,
		);
	}

	/** An amount above capacity waits for a full bucket and then goes into debt. */
	delayMs(amount: number, now: number): number {
		this.refill(now);
		const needed = Math.min(amount, this.capacity);
		if (this.tokens >= needed) return 0;
		return Math.max(
			1,
			Math.ceil(((needed - this.tokens) / this.refillPerSecond) * 1000),
		);
	}

	take(amount: number, now: number) {
		this.refill(now);
		this.tokens -= amount;
	}

	full(now: number): boolean {
		this.refill(now);
		return this.tokens >= this.capacity;
	}
}

class FrameBudget {
	private readonly frames: TokenBucket;
	private readonly bytes: TokenBucket;

	constructor(limits: FrameBudgetLimits, now: number) {
		this.frames = new TokenBucket(limits.frames, limits.framesPerSecond, now);
		this.bytes = new TokenBucket(limits.bytes, limits.bytesPerSecond, now);
	}

	delayMs(bytes: number, now: number): number {
		return Math.max(
			this.frames.delayMs(1, now),
			this.bytes.delayMs(bytes, now),
		);
	}

	take(bytes: number, now: number) {
		this.frames.take(1, now);
		this.bytes.take(bytes, now);
	}

	idle(now: number): boolean {
		return this.frames.full(now) && this.bytes.full(now);
	}
}

/** Budgets are keyed by subject and outlive sockets, so reconnecting never refills a burst. */
export class FrameBudgets {
	private readonly budgets = new Map<string, FrameBudget>();

	/** Returns 0 after charging every budget, otherwise how long to wait before asking again. */
	reserve(
		keys: readonly (readonly [key: string, limits: FrameBudgetLimits])[],
		bytes: number,
		now = Date.now(),
	): number {
		const budgets = keys.map(([key, limits]) => {
			let budget = this.budgets.get(key);
			if (!budget) {
				budget = new FrameBudget(limits, now);
				this.budgets.set(key, budget);
			}
			return budget;
		});
		const delay = Math.max(0, ...budgets.map((b) => b.delayMs(bytes, now)));
		if (delay === 0) for (const budget of budgets) budget.take(bytes, now);
		return delay;
	}

	sweep(now = Date.now()) {
		for (const [key, budget] of this.budgets)
			if (budget.idle(now)) this.budgets.delete(key);
	}

	get size(): number {
		return this.budgets.size;
	}
}

export type OutboxFrame = {
	target: string;
	bytes: number;
	deliver: () => Promise<void>;
};

export type OutboxLimits = {
	pendingBytes: number;
	pendingBytesPerTarget: number;
};

export const DEVICE_OUTBOX_LIMITS: OutboxLimits = {
	pendingBytes: MIB,
	pendingBytesPerTarget: 256 * 1024,
};
export const CONTROLLER_OUTBOX_LIMITS: OutboxLimits = {
	pendingBytes: 256 * 1024,
	pendingBytesPerTarget: 256 * 1024,
};

/**
 * Holds a socket's over-budget frames instead of closing it. Each target drains
 * in order on its own, so one throttled participant never delays another.
 */
export class ManagementOutbox {
	private readonly queues = new Map<
		string,
		{ frames: OutboxFrame[]; bytes: number }
	>();
	private pending = 0;
	private closed = false;

	constructor(
		private readonly admit: (frame: OutboxFrame, now: number) => number,
		private readonly limits: OutboxLimits,
		private readonly sleep: (ms: number) => Promise<unknown> = (ms) =>
			Bun.sleep(ms),
		private readonly now: () => number = () => Date.now(),
	) {}

	/** False when the frame would exceed a pending bound; the caller chooses the penalty. */
	enqueue(frame: OutboxFrame): boolean {
		const queue = this.queues.get(frame.target);
		if (
			this.closed ||
			this.pending + frame.bytes > this.limits.pendingBytes ||
			(queue?.bytes ?? 0) + frame.bytes > this.limits.pendingBytesPerTarget
		)
			return false;
		this.pending += frame.bytes;
		if (queue) {
			queue.frames.push(frame);
			queue.bytes += frame.bytes;
			return true;
		}
		const created = { frames: [frame], bytes: frame.bytes };
		this.queues.set(frame.target, created);
		void this.drain(frame.target, created);
		return true;
	}

	close() {
		this.closed = true;
		this.queues.clear();
		this.pending = 0;
	}

	get pendingBytes(): number {
		return this.pending;
	}

	private async drain(
		target: string,
		queue: { frames: OutboxFrame[]; bytes: number },
	) {
		while (!this.closed) {
			const frame = queue.frames[0];
			if (!frame) break;
			const delay = this.admit(frame, this.now());
			if (delay > 0) {
				await this.sleep(delay);
				continue;
			}
			queue.frames.shift();
			try {
				await frame.deliver();
			} catch {
				// Delivery owns its own failure handling, including closing the socket.
			} finally {
				queue.bytes -= frame.bytes;
				if (!this.closed) this.pending -= frame.bytes;
			}
		}
		if (this.queues.get(target) === queue) this.queues.delete(target);
	}
}

/** An owner-signed policy names at most 24 grantee accounts, plus the owner. */
const MAX_DEVICE_ACCOUNTS = 25;
export const MAX_CONTROLLERS_PER_DEVICE_ACCOUNT = 8;
/** Sized so no set of other accounts can take every slot and lock out the owner. */
export const MAX_CONTROLLERS_PER_DEVICE =
	MAX_DEVICE_ACCOUNTS * MAX_CONTROLLERS_PER_DEVICE_ACCOUNT;
/** Device-role sockets replace each other, so this bounds an account's controllers. */
const MAX_CONTROLLERS_PER_ACCOUNT = 128;

export type SlotAdmission = {
	role: "device" | "controller";
	deviceId: string;
	subject: string;
	tokenId: string;
};

/** A management socket's `subject` is already scoped to its device and role. */
export function connectionSlotsFor(
	subject: string | null,
	subjectLimit: number,
	management: SlotAdmission | null,
): [string, number][] {
	const controller = management?.role === "controller" ? management : null;
	const slots: [string, number][] =
		subject === null
			? []
			: [
					[
						subject,
						controller
							? Math.min(subjectLimit, MAX_CONTROLLERS_PER_DEVICE_ACCOUNT)
							: subjectLimit,
					],
				];
	if (controller)
		slots.push(
			[`device-signaling-token:${controller.tokenId}`, 1],
			[
				`device-signaling-controllers:${controller.deviceId}`,
				MAX_CONTROLLERS_PER_DEVICE,
			],
			[
				`device-signaling-account:${controller.subject}`,
				MAX_CONTROLLERS_PER_ACCOUNT,
			],
		);
	return slots;
}

/** Admits a connection only when every one of its slots has room. */
export class ConnectionSlots {
	private readonly live = new Map<string, number>();

	acquire(slots: readonly (readonly [key: string, limit: number])[]): boolean {
		if (slots.some(([key, limit]) => (this.live.get(key) ?? 0) >= limit))
			return false;
		for (const [key] of slots)
			this.live.set(key, (this.live.get(key) ?? 0) + 1);
		return true;
	}

	release(keys: readonly string[]) {
		for (const key of keys) {
			const live = this.live.get(key);
			if (live === undefined) continue;
			if (live <= 1) this.live.delete(key);
			else this.live.set(key, live - 1);
		}
	}

	count(key: string): number {
		return this.live.get(key) ?? 0;
	}
}

/** Counts discarded frames by reason so the relay reports them without payloads. */
export class DiscardCounter {
	private readonly counts = new Map<string, number>();

	add(reason: string) {
		this.counts.set(reason, (this.counts.get(reason) ?? 0) + 1);
	}

	drain(): string | null {
		if (this.counts.size === 0) return null;
		const summary = [...this.counts]
			.map(([reason, count]) => `${reason}=${count}`)
			.join(", ");
		this.counts.clear();
		return summary;
	}
}
