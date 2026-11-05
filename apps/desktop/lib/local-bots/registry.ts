import { LOCAL_NOT_FOUND, errorCode, localBots, localInbox } from "./commands";

/** Whether the store holds the id. A store that can't answer rejects, so nothing is guessed. */
type Probe = (id: string) => Promise<boolean>;

/**
 * Which ids of one kind live on this computer. Complete local listings and
 * local creates fill it; an id it has not seen is probed once. Ids known to be
 * the account's skip the probe for as long as the same account is signed in,
 * and so do local ids that moved there, which stand for their account ids.
 */
export class LocalIdRegistry {
	private readonly local = new Set<string>();
	private readonly remote = new Set<string>();
	private readonly moved = new Map<string, string>();
	private remoteOwner?: string;

	constructor(private readonly probe: Probe) {}

	/** A complete local listing replaces what was known; a listing older than a move can't bring its ids back. */
	setLocal(ids: Iterable<string>): void {
		this.local.clear();
		for (const id of ids) {
			if (!this.moved.has(id)) this.local.add(id);
		}
	}

	addLocal(id: string): void {
		this.local.add(id);
	}

	forgetLocal(id: string): void {
		this.local.delete(id);
	}

	addRemote(owner: string, ids: Iterable<string>): void {
		this.ownedBy(owner);
		for (const id of ids) this.remote.add(id);
	}

	/** After a move: calls naming the local id go to `owner`'s account as `accountId`. */
	moveToAccount(owner: string, localId: string, accountId: string): void {
		this.ownedBy(owner);
		this.local.delete(localId);
		this.moved.set(localId, accountId);
		this.remote.add(accountId);
	}

	/** The id `owner`'s account knows this one by. */
	accountId(id: string, owner: string): string {
		return (this.remoteOwner === owner && this.moved.get(id)) || id;
	}

	/**
	 * A moved id is the account's; otherwise a copy on this computer wins over a remote mark.
	 * Rejects when the store can't say, so a local id's call never reaches the account.
	 */
	async isLocal(id: string, owner: string): Promise<boolean> {
		const known = this.remoteOwner === owner;
		if (known && this.moved.has(id)) return false;
		if (this.local.has(id)) return true;
		if (known && this.remote.has(id)) return false;
		const here = await this.probe(id);
		if (here) this.local.add(id);
		else this.addRemote(owner, [id]);
		return here;
	}

	/** Runs a call on this computer's copy; a store that no longer holds the id drops its mark, so the next call asks again. */
	async onLocal<T>(id: string, call: () => Promise<T>): Promise<T> {
		try {
			return await call();
		} catch (error) {
			if (errorCode(error) === LOCAL_NOT_FOUND) this.local.delete(id);
			throw error;
		}
	}

	private ownedBy(owner: string): void {
		if (this.remoteOwner === owner) return;
		this.remote.clear();
		this.moved.clear();
		this.remoteOwner = owner;
	}
}

export interface ILocalRegistries {
	readonly bots: LocalIdRegistry;
	readonly conversations: LocalIdRegistry;
}

const registries = new WeakMap<object, ILocalRegistries>();

/** One registry pair per backend, shared by its bot and inbox states and the move. */
export function localRegistries(backend: object): ILocalRegistries {
	let found = registries.get(backend);
	if (!found) {
		found = {
			bots: new LocalIdRegistry(
				async (id) => (await localBots.get(id)) !== null,
			),
			conversations: new LocalIdRegistry(
				async (id) => (await localInbox.conversation(id)) !== null,
			),
		};
		registries.set(backend, found);
	}
	return found;
}

export function idsOf(records: readonly { id: string }[]): string[] {
	return records.map((record) => record.id);
}
