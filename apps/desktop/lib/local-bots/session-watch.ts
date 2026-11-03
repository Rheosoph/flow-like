import { type ILocalBotsBackend, accountSession } from "./account";

export interface ISessionSnapshot {
	/** The account the bot and inbox states call; null sends everything to this computer. */
	readonly routing: string | null;
	/** Who is signed in, kept while a token renews; undefined while that is not known yet. */
	readonly person: string | null | undefined;
}

export function sessionSnapshot(backend: ILocalBotsBackend): ISessionSnapshot {
	const auth = backend.auth;
	return {
		routing: accountSession(backend)?.sub ?? null,
		person:
			auth && !auth.isLoading ? (auth.user?.profile.sub ?? null) : undefined,
	};
}

export interface ISessionChange {
	/** The states now answer from somewhere else, so cached bot and inbox reads are stale. */
	readonly refetch: boolean;
	/** The sub someone signed in as since the last observation; a restored session is not one. */
	readonly signedIn?: string;
}

/** Follows the session across renders. The first observation only sets the baseline. */
export class SessionWatch {
	private routing?: string | null;
	private person?: string | null;

	observe(next: ISessionSnapshot): ISessionChange {
		const refetch = this.routing !== undefined && this.routing !== next.routing;
		this.routing = next.routing;
		if (next.person === undefined) return { refetch };
		const signedIn =
			this.person !== undefined &&
			next.person !== null &&
			next.person !== this.person
				? next.person
				: undefined;
		this.person = next.person;
		return { refetch, signedIn };
	}
}
