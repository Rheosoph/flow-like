import { type UserManagerSettings, WebStorageStateStore } from "oidc-client-ts";
import { getPublicWebConfig } from "./public-config";

/** Account and hosted pages share the same OIDC session and callback. */
export function getWebOidcSettings(
	config: UserManagerSettings,
): UserManagerSettings {
	const publicConfig = getPublicWebConfig();
	return {
		...config,
		...(publicConfig.redirectUrl
			? { redirect_uri: publicConfig.redirectUrl }
			: {}),
		...(publicConfig.logoutUrl
			? { post_logout_redirect_uri: publicConfig.logoutUrl }
			: {}),
		userStore: new WebStorageStateStore({ store: localStorage }),
		automaticSilentRenew: true,
	};
}
