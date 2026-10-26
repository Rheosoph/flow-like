import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import type { AuthContextProps } from "react-oidc-context";

const backendIds = new WeakMap<object, number>();
let nextBackendId = 0;

/** Shared by auth initialization and its observers; never persisted across sessions. */
export function accountQueryDeps(
	userState: object,
	auth: Pick<AuthContextProps, "user" | "isAuthenticated">,
) {
	let backendId = backendIds.get(userState);
	if (backendId === undefined) {
		backendId = ++nextBackendId;
		backendIds.set(userState, backendId);
	}
	return [
		{
			account: {
				backendId,
				hub: getApiOrigin(),
				issuer: auth.user?.profile.iss ?? null,
				subject: auth.user?.profile.sub ?? null,
				authenticated: auth.isAuthenticated,
			},
		},
	];
}
