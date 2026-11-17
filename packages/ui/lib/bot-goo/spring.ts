export interface SpringConfig {
	stiffness: number;
	damping: number;
}

/** Wobbles a little: the den, landing bots, the prompt's morph. */
export const BOUNCY: SpringConfig = { stiffness: 420, damping: 18 };
/** Follows the pointer closely without jitter. */
export const TIGHT: SpringConfig = { stiffness: 1400, damping: 60 };
/** Window following: smooth, no overshoot. */
export const SMOOTH: SpringConfig = { stiffness: 520, damping: 46 };
/** A landing's wobble: rings a few times before it settles. */
export const JELLY: SpringConfig = { stiffness: 360, damping: 8 };

const STEP = 1 / 240;
const MAX_FRAME = 1 / 20;

export class Spring {
	value: number;
	velocity = 0;
	target: number;

	constructor(
		value: number,
		public config: SpringConfig = BOUNCY,
	) {
		this.value = value;
		this.target = value;
	}

	set(target: number, config?: SpringConfig): void {
		this.target = target;
		if (config) this.config = config;
	}

	/** Jumps there without moving. */
	snap(value: number): void {
		this.value = value;
		this.target = value;
		this.velocity = 0;
	}

	/** Advances `seconds`; true while it still moves. */
	step(seconds: number): boolean {
		let left = Math.min(seconds, MAX_FRAME);
		while (left > 0) {
			const dt = Math.min(STEP, left);
			const force =
				-this.config.stiffness * (this.value - this.target) -
				this.config.damping * this.velocity;
			this.velocity += force * dt;
			this.value += this.velocity * dt;
			left -= dt;
		}
		if (this.resting()) {
			this.value = this.target;
			this.velocity = 0;
			return false;
		}
		return true;
	}

	resting(): boolean {
		return (
			Math.abs(this.value - this.target) < 0.01 &&
			Math.abs(this.velocity) < 0.05
		);
	}
}

/** Two springs that move together, for a point. */
export class Spring2 {
	readonly x: Spring;
	readonly y: Spring;

	constructor(x: number, y: number, config: SpringConfig = BOUNCY) {
		this.x = new Spring(x, config);
		this.y = new Spring(y, config);
	}

	set(x: number, y: number, config?: SpringConfig): void {
		this.x.set(x, config);
		this.y.set(y, config);
	}

	snap(x: number, y: number): void {
		this.x.snap(x);
		this.y.snap(y);
	}

	step(seconds: number): boolean {
		const moved = this.x.step(seconds);
		return this.y.step(seconds) || moved;
	}
}
