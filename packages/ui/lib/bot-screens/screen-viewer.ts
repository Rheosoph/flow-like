import type { IBotComputerView } from "../../state/backend-state/bot-state";
import { toBase64 } from "./base64";
import {
	type PlaygroundControl,
	PlaygroundDecoder,
	type PlaygroundEvent,
	type PlaygroundFrameMeta,
	type PlaygroundState,
	encodeControl,
} from "./playground-codec";
import type { ScreenStream } from "./screen-stream";

export const ACTIVE_FOR_MS = 180_000;
export const NOTICE_FOR_MS = 8_000;

interface HeldFrame {
	meta: PlaygroundFrameMeta;
	image: string;
	received: number;
}

interface HeldNotice {
	text: string;
	at: number;
}

const ASLEEP: IBotComputerView = {
	state: "asleep",
	kind: null,
	caption: null,
	image: null,
	age_ms: null,
};

/**
 * Reads a playground stream, keeps the latest state and picture and shows them as the Computer
 * tab's view, the way the desktop's `Live::view` does for a device tunnel.
 */
export class ScreenViewer {
	readonly ended: Promise<string>;
	private readonly now: () => number;
	private readonly listeners = new Set<(view: IBotComputerView) => void>();
	private state: PlaygroundState | null = null;
	private frame: HeldFrame | null = null;
	private notice: HeldNotice | null = null;
	private endReason: string | null = null;
	private finish!: (reason: string) => void;
	private timer: ReturnType<typeof setTimeout> | null = null;
	private writes: Promise<unknown> = Promise.resolve();

	constructor(
		private readonly stream: ScreenStream,
		options: { now?: () => number } = {},
	) {
		this.now = options.now ?? Date.now;
		this.ended = new Promise<string>((resolve) => {
			this.finish = resolve;
		});
		void this.pump();
	}

	get view(): IBotComputerView {
		const now = this.now();
		const notice =
			this.notice && now - this.notice.at < NOTICE_FOR_MS
				? this.notice.text
				: null;
		const taken_over = this.state?.taken_over ?? false;
		const frame = this.frame;
		const age = frame ? frame.meta.age_ms + (now - frame.received) : null;
		if (!frame || this.state?.asleep || age === null || age > ACTIVE_FOR_MS) {
			return { ...ASLEEP, taken_over, can_control: false, notice };
		}
		const computer = frame.meta.kind === "computer";
		return {
			state: "active",
			kind: frame.meta.kind,
			caption: frame.meta.caption,
			image: frame.image,
			age_ms: age,
			observation: (frame.meta.observation ??
				null) as IBotComputerView["observation"],
			taken_over,
			can_control:
				computer && this.state !== null && this.state.display !== "none",
			width: frame.meta.width,
			height: frame.meta.height,
			notice,
		};
	}

	subscribe(listener: (view: IBotComputerView) => void): () => void {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	}

	send(control: PlaygroundControl): Promise<void> {
		if (this.endReason !== null) {
			return Promise.reject(
				new Error(`The live view of this bot ended: ${this.endReason}`),
			);
		}
		let bytes: Uint8Array;
		try {
			bytes = encodeControl(control);
		} catch (error) {
			return Promise.reject(error);
		}
		this.notice = null;
		const write = this.writes.then(() => this.stream.write(bytes));
		this.writes = write.catch(() => {});
		return write;
	}

	close(): void {
		if (this.endReason !== null) return;
		this.end("closed");
		this.stream.reset();
	}

	private async pump(): Promise<void> {
		const decoder = new PlaygroundDecoder();
		try {
			for (;;) {
				const chunk = await this.stream.read();
				if (chunk === null) return this.end("closed");
				decoder.push(chunk);
				for (let event = decoder.next(); event; event = decoder.next()) {
					this.apply(event);
				}
			}
		} catch (error) {
			this.end(error instanceof Error ? error.message : String(error));
			this.stream.reset();
		}
	}

	private apply(event: PlaygroundEvent): void {
		const now = this.now();
		if (event.type === "state") this.state = event.state;
		else if (event.type === "error")
			this.notice = { text: `${event.code}: ${event.message}`, at: now };
		else {
			this.frame = {
				meta: event.meta,
				image: `data:image/jpeg;base64,${toBase64(event.jpeg)}`,
				received: now,
			};
		}
		this.publish();
	}

	private publish(): void {
		if (this.endReason !== null) return;
		this.scheduleExpiry();
		const view = this.view;
		for (const listener of this.listeners) listener(view);
	}

	private scheduleExpiry(): void {
		if (this.timer) clearTimeout(this.timer);
		const now = this.now();
		const deadlines: number[] = [];
		if (this.notice) deadlines.push(this.notice.at + NOTICE_FOR_MS - now);
		if (this.frame)
			deadlines.push(
				ACTIVE_FOR_MS - this.frame.meta.age_ms - (now - this.frame.received),
			);
		const next = Math.min(...deadlines.filter((ms) => ms > 0));
		this.timer = Number.isFinite(next)
			? setTimeout(() => this.publish(), next + 1)
			: null;
	}

	private end(reason: string): void {
		if (this.endReason !== null) return;
		this.endReason = reason;
		if (this.timer) clearTimeout(this.timer);
		this.timer = null;
		this.finish(reason);
	}
}
