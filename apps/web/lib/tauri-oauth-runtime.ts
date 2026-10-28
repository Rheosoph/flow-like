import { useBackendStore } from "@flow-like/flow-like-ui";
import type { AuthContextProps } from "react-oidc-context";
import { type OAuthAccount, createWebOAuthRuntime } from "./web-oauth-runtime";

/** The signed-in account of the app backend. */
export function backendOAuthAccount(): OAuthAccount | undefined {
	const backend = useBackendStore.getState().backend as {
		profile?: { hub?: string };
		auth?: AuthContextProps;
	} | null;
	return backend
		? { hub: backend.profile?.hub, auth: backend.auth }
		: undefined;
}

export const webOAuthRuntime = createWebOAuthRuntime(backendOAuthAccount);

export const tauriOAuthRuntime = webOAuthRuntime;
