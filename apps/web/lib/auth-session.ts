import type { AuthContextProps } from "react-oidc-context";

const rejectedTokens = new WeakMap<
	AuthContextProps["signinRedirect"],
	Set<string | undefined>
>();
const listeners = new Set<() => void>();

/** Keep the token used by the request so a late 401 cannot reject a newer login. */
export function reportUnauthorized(
	auth: AuthContextProps | undefined,
	accessToken: string | undefined,
): void {
	if (!auth) return;
	let tokens = rejectedTokens.get(auth.signinRedirect);
	if (tokens?.has(accessToken)) return;
	if (!tokens) {
		tokens = new Set();
		rejectedTokens.set(auth.signinRedirect, tokens);
	}
	tokens.add(accessToken);
	for (const listener of listeners) listener();
}

export function isSessionRejected(
	auth: AuthContextProps,
	accessToken: string | undefined,
): boolean {
	return rejectedTokens.get(auth.signinRedirect)?.has(accessToken) ?? false;
}

export function subscribeSessionRejection(listener: () => void): () => void {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}
