import { configureOAuthSession } from "@flow-like/flow-like-ui";
import { backendOAuthAccount } from "./tauri-oauth-runtime";
import { oauthAccountSession } from "./web-oauth-runtime";

configureOAuthSession(() => oauthAccountSession(backendOAuthAccount()));

export {
	oauthDB,
	oauthConsentStore,
	oauthTokenStore,
} from "@flow-like/flow-like-ui";
