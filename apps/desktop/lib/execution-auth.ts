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
	private opening?: Promise<string>;
	private nativeSessionId?: string;

	constructor(
		private readonly openSession: () => Promise<string>,
		private readonly send: (update: ExecutionAuthUpdate) => Promise<void>,
	) {}

	get sessionId(): string {
		if (!this.nativeSessionId) {
			throw new Error("Native execution session is not ready.");
		}
		return this.nativeSessionId;
	}

	private session(): Promise<string> {
		this.opening ??= this.openSession()
			.then((sessionId) => {
				if (!sessionId) throw new Error("Native execution session is missing.");
				this.nativeSessionId = sessionId;
				return sessionId;
			})
			.catch((error: unknown) => {
				this.opening = undefined;
				throw error;
			});
		return this.opening;
	}

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
		const update = { ...snapshot, sequence };
		this.pending = this.pending
			.catch(() => undefined)
			.then(async () => {
				const sessionId = await this.session();
				await this.send({ ...update, sessionId });
			})
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
