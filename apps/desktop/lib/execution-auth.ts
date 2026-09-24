export interface ExecutionAuthSnapshot {
	hub: string;
	subject: string | null;
	token: string | null;
}

export interface ExecutionAuthUpdate extends ExecutionAuthSnapshot {
	sessionId: string;
	sequence: number;
}

/** Keeps native execution credentials in the same session as the desktop UI. */
export class ExecutionAuthBridge {
	private pending: Promise<void> = Promise.resolve();
	private snapshot?: ExecutionAuthSnapshot;
	private sequence = 0;

	constructor(
		readonly sessionId: string,
		private readonly send: (update: ExecutionAuthUpdate) => Promise<void>,
	) {}

	update(snapshot: ExecutionAuthSnapshot): Promise<void> {
		if (
			this.snapshot?.hub === snapshot.hub &&
			this.snapshot.subject === snapshot.subject &&
			this.snapshot.token === snapshot.token
		) {
			return this.pending;
		}
		this.snapshot = { ...snapshot };
		const sequence = ++this.sequence;
		const update = { ...snapshot, sessionId: this.sessionId, sequence };
		this.pending = this.pending
			.catch(() => undefined)
			.then(() => this.send(update))
			.catch((error: unknown) => {
				// Permit retry without letting an older failed update erase a newer one.
				if (sequence === this.sequence) this.snapshot = undefined;
				throw error;
			});
		return this.pending;
	}

	async ready(): Promise<void> {
		let pending: Promise<void>;
		do {
			pending = this.pending;
			await pending;
		} while (pending !== this.pending);
	}
}
