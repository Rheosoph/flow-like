import type { IProfile } from "@flow-like/flow-like-ui";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import {
	renewOidcSession,
	requiresInteractiveSignIn,
} from "@flow-like/flow-like-ui/lib/auth/oidc-renewal";
import type { User } from "oidc-client-ts";
import type { AuthContextProps } from "react-oidc-context";

interface HubSession {
	hub?: string;
	user?: User | null;
	contextUser?: User | null;
	renewing?: Promise<User>;
	epoch: number;
	active: boolean;
}

const sessions = new WeakMap<object, HubSession>();

function sessionFor(auth: AuthContextProps): HubSession {
	const key = auth.events ?? auth;
	let session = sessions.get(key);
	if (!session) {
		session = {
			user: auth.user,
			contextUser: auth.user,
			epoch: 0,
			active: true,
		};
		sessions.set(key, session);
		const current = session;
		auth.events?.addUserLoaded((user) => {
			if (current.user?.profile.sub !== user.profile.sub) current.epoch++;
			current.user = user;
		});
		auth.events?.addUserUnloaded(() => {
			current.epoch++;
			current.user = null;
		});
	}
	return session;
}

/** Bind the session before exposing it to consumers of the current profile. */
export function bindHubAuth(auth: AuthContextProps, hub: string): boolean {
	const session = sessionFor(auth);
	if (session.hub !== undefined && session.hub !== hub) {
		setHubAuthActive(auth, false);
		return false;
	}
	session.hub = hub;
	if (session.contextUser !== auth.user) {
		session.contextUser = auth.user;
		session.user = auth.user;
	}
	return true;
}

/** The origin bound to these tokens, including while profile setup is pending. */
export function hubAuthOrigin(
	auth: AuthContextProps | null | undefined,
): string | undefined {
	return auth ? sessionFor(auth).hub : undefined;
}

export function authForHub(
	profile: Partial<IProfile> | null | undefined,
	auth?: AuthContextProps | null,
): AuthContextProps | undefined {
	if (!auth) return undefined;
	const { hub, active } = sessionFor(auth);
	return active && (hub === undefined || hub === getApiOrigin(profile))
		? auth
		: undefined;
}

export function setHubAuthActive(
	auth: AuthContextProps,
	active: boolean,
): void {
	const session = sessionFor(auth);
	if (session.active !== active) session.epoch++;
	session.active = active;
}

export function hubAuthUser(auth: AuthContextProps): User | null | undefined {
	return sessionFor(auth).user;
}

export function hubAuthHeaders(
	input: HeadersInit | undefined,
	token?: string,
): Record<string, string> {
	const inputHeaders = new Headers(input);
	inputHeaders.delete("authorization");
	const headers = Object.fromEntries(inputHeaders);
	if (token) headers.Authorization = `Bearer ${token}`;
	return headers;
}

export async function renewHubAuth(auth: AuthContextProps): Promise<User> {
	const session = sessionFor(auth);
	if (!session.active) throw new Error("The Hub session is no longer active.");
	if (!session.renewing) {
		const epoch = session.epoch;
		const hub = session.hub;
		session.renewing = Promise.resolve()
			.then(() => renewOidcSession(auth))
			.then((user) => {
				if (!user?.access_token || user.expired || epoch !== session.epoch)
					throw new Error("Sign in again to continue using this hub.");
				session.user = user;
				return user;
			})
			.catch(async (error) => {
				if (
					requiresInteractiveSignIn(error) &&
					session.active &&
					epoch === session.epoch
				) {
					session.user = null;
					await auth.removeUser();
					if (session.active && session.hub === hub && !session.user)
						await auth.signinRedirect();
				}
				throw error;
			})
			.finally(() => {
				session.renewing = undefined;
			});
	}
	return session.renewing;
}

/** Renew before expiry and retry an authentication refusal once, before a body is consumed. */
export async function withHubAuth(
	profile: IProfile,
	auth: AuthContextProps | null | undefined,
	send: (token?: string) => Promise<Response>,
): Promise<Response> {
	const scoped = authForHub(profile, auth);
	const session = scoped ? sessionFor(scoped) : undefined;
	const epoch = session?.epoch;
	let user = session?.user;
	if (
		scoped &&
		user &&
		(user.expired ||
			(user.expires_at !== undefined &&
				user.expires_at <= Date.now() / 1000 + 30))
	) {
		user = await renewHubAuth(scoped);
	}
	const token = user?.access_token;
	const response = await send(token);
	if (response.status !== 401 || !scoped || !token || session?.epoch !== epoch)
		return response;
	const latest = session?.user;
	if (!latest) return response;
	const renewed =
		latest.access_token !== token && !latest.expired
			? latest
			: await renewHubAuth(scoped);
	await response.body?.cancel();
	return send(renewed.access_token);
}
