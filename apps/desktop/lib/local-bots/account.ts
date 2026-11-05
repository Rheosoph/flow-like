import type { IBot, IProfile } from "@flow-like/flow-like-ui";
import { asArray } from "@flow-like/flow-like-ui/lib/response-shape";
import type { AuthContextProps } from "react-oidc-context";
import { get } from "../api";
import { isHubUnavailable } from "../api-error";
import { authForHub, hubAuthUser } from "../hub-auth";

/** What the local bot layer reads from the desktop backend. */
export interface ILocalBotsBackend {
	readonly profile?: IProfile;
	readonly auth?: AuthContextProps;
}

/** A hub account the composite bot and inbox states can call. */
export interface IAccountSession {
	readonly profile: IProfile;
	readonly auth: AuthContextProps;
	readonly sub: string;
}

/**
 * Signed in the way the other desktop states decide it: a profile and an
 * authenticated session with an access token. Without one, bots and
 * conversations live on this computer only.
 */
export function accountSession(
	backend: ILocalBotsBackend,
): IAccountSession | undefined {
	const { profile, auth } = backend;
	const sub = auth?.user?.profile.sub;
	if (!profile || !auth?.isAuthenticated || !auth.user?.access_token || !sub)
		return undefined;
	return { profile, auth, sub };
}

/** A call about one record of the account, under the id the account knows it by. */
export interface IAccountRoute extends IAccountSession {
	readonly id: string;
}

/** The token the profile's hub takes now: the API helpers keep the renewed one. */
export function hubAccessToken(session: IAccountSession): string | undefined {
	const scoped = authForHub(session.profile, session.auth);
	return scoped ? hubAuthUser(scoped)?.access_token : undefined;
}

export function botPath(botId: string): string {
	return `bots/${encodeURIComponent(botId)}`;
}

export async function listAccountBots(
	session: IAccountSession,
): Promise<IBot[]> {
	const response = await get<{ bots?: IBot[] }>(
		session.profile,
		"bots",
		session.auth,
	);
	return asArray(response?.bots);
}

/**
 * The account's half of a merged list as last read this session, so the
 * local half keeps refreshing while the hub can't be reached. Before a first
 * read, or on a refusal, the error stands and the cached query keeps its data.
 */
export class AccountListReads<T> {
	private readonly last = new Map<string, T[]>();
	private readonly revisions = new Map<string, number>();

	/** A successful deletion also removes the item from the offline fallback. */
	remove(key: string, matches: (item: T) => boolean): void {
		this.last.set(
			key,
			(this.last.get(key) ?? []).filter((item) => !matches(item)),
		);
		this.revisions.set(key, (this.revisions.get(key) ?? 0) + 1);
	}

	async read(key: string, load: () => Promise<T[]>): Promise<T[]> {
		const revision = this.revisions.get(key) ?? 0;
		try {
			const items = await load();
			if ((this.revisions.get(key) ?? 0) !== revision) {
				return this.last.get(key) ?? [];
			}
			this.last.set(key, items);
			return items;
		} catch (error) {
			const known = this.last.get(key);
			if (!known || !isHubUnavailable(error)) throw error;
			console.warn(
				"[LocalBots] Hub unreachable; showing its last list:",
				error,
			);
			return known;
		}
	}
}
