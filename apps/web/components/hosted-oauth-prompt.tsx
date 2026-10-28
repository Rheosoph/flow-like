"use client";

import { DeviceFlowDialog } from "@flow-like/flow-like-ui/components/oauth/device-flow-dialog";
import { OAuthConsentDialog } from "@flow-like/flow-like-ui/components/oauth/oauth-consent-dialog";
import { getMissingOAuthScopes } from "@flow-like/flow-like-ui/lib/oauth/token-scopes";
import type {
	IOAuthProvider,
	IStoredOAuthToken,
} from "@flow-like/flow-like-ui/lib/oauth/types";
import { useTranslation } from "@flow-like/locales";
import {
	useCallback,
	useEffect,
	useReducer,
	useState,
	useSyncExternalStore,
} from "react";
import { toast } from "sonner";
import type { HostedOAuth, HostedOAuthRequest } from "../lib/hosted-oauth";
import {
	OAUTH_CALLBACK_CHANNEL,
	parseOAuthCallbackCompletion,
} from "../lib/oauth-callback-storage";

interface Connections {
	authorized: Set<string>;
	missingScopes: Record<string, string[]>;
}

type ConnectionUpdate =
	| { type: "started"; providerId: string }
	| { type: "finished"; providerId: string; missingScopes: string[] };

const NO_CONNECTIONS: Connections = {
	authorized: new Set(),
	missingScopes: {},
};

/** A sign-in counts once its provider granted every scope the run needs. */
function reduceConnections(
	state: Connections,
	update: ConnectionUpdate,
): Connections {
	const missing = update.type === "finished" ? update.missingScopes : [];
	const missingScopes = {
		...state.missingScopes,
		[update.providerId]: missing,
	};
	if (update.type === "started") return { ...state, missingScopes };
	const authorized = new Set(state.authorized);
	if (missing.length === 0) authorized.add(update.providerId);
	else authorized.delete(update.providerId);
	return { authorized, missingScopes };
}

function providerIn(
	request: HostedOAuthRequest,
	id: string,
): IOAuthProvider | undefined {
	for (const provider of request.providers)
		if (provider.id === id) return provider;
	return undefined;
}

/** A sign-in finishes in the callback tab, which announces it to the visitor's other tabs. */
function useOAuthCompletions(
	oauth: HostedOAuth,
	onToken: (token: IStoredOAuthToken) => void,
) {
	useEffect(() => {
		let channel: BroadcastChannel;
		try {
			channel = new BroadcastChannel(OAUTH_CALLBACK_CHANNEL);
		} catch {
			return;
		}
		function receive(event: MessageEvent) {
			if (event.data?.type !== "oauth-complete") return;
			const completion = parseOAuthCallbackCompletion(
				JSON.stringify(event.data.payload),
			);
			const visitor = oauth.sessionKey();
			if (visitor && completion?.pending.binding?.sessionKey === visitor)
				onToken(completion.token);
		}
		channel.addEventListener("message", receive);
		return () => {
			channel.removeEventListener("message", receive);
			channel.close();
		};
	}, [oauth, onToken]);
}

/** The prompt's providers the visitor connected so far, and how they connect another. */
function useConnections(oauth: HostedOAuth, request: HostedOAuthRequest) {
	const [connections, update] = useReducer(reduceConnections, NO_CONNECTIONS);
	const [deviceProvider, setDeviceProvider] = useState<IOAuthProvider | null>(
		null,
	);
	const finished = useCallback(
		(token: IStoredOAuthToken) => {
			const provider = providerIn(request, token.providerId);
			if (!provider) return;
			update({
				type: "finished",
				providerId: provider.id,
				missingScopes: getMissingOAuthScopes(
					token,
					provider.merged_scopes ?? provider.scopes,
				),
			});
		},
		[request],
	);
	useOAuthCompletions(oauth, finished);
	const authorize = useCallback(
		async (providerId: string) => {
			const provider = providerIn(request, providerId);
			if (!provider) return;
			update({ type: "started", providerId });
			if (provider.use_device_flow && provider.device_auth_url) {
				setDeviceProvider(provider);
				return;
			}
			try {
				await oauth.service.startAuthorization(provider, {
					appId: oauth.appId,
				});
			} catch (error) {
				toast.error(error instanceof Error ? error.message : String(error));
			}
		},
		[oauth, request],
	);
	const deviceFlowEnded = useCallback(
		(token?: IStoredOAuthToken) => {
			setDeviceProvider(null);
			if (token) finished(token);
		},
		[finished],
	);
	return { connections, deviceProvider, authorize, deviceFlowEnded };
}

function ConnectPrompt({
	oauth,
	request,
}: { oauth: HostedOAuth; request: HostedOAuthRequest }) {
	const { t } = useTranslation("interfaces");
	const { connections, deviceProvider, authorize, deviceFlowEnded } =
		useConnections(oauth, request);
	const skip = useCallback(() => {
		request.answer("skip");
	}, [request]);
	const connect = useCallback(
		(remember: boolean) => {
			request.answer("connect", remember);
		},
		[request],
	);
	const dismiss = useCallback(
		(open: boolean) => {
			if (!open) skip();
		},
		[skip],
	);
	return (
		<>
			<OAuthConsentDialog
				open
				onOpenChange={dismiss}
				providers={request.providers}
				onAuthorize={authorize}
				onConfirmAll={connect}
				onCancel={skip}
				authorizedProviders={connections.authorized}
				preAuthorizedProviders={request.connected}
				missingScopesByProvider={connections.missingScopes}
				skip={{
					label: t("hostedOAuth.skip", "Skip"),
					description: t(
						"hostedOAuth.skipDescription",
						"Connect your own accounts, or skip to use the sign-ins saved for this app.",
					),
				}}
			/>
			<DeviceFlowDialog
				provider={deviceProvider}
				oauthService={oauth.service}
				runtime={oauth.runtime}
				onSuccess={deviceFlowEnded}
				onCancel={deviceFlowEnded}
			/>
		</>
	);
}

/** The in-app connect prompt for a signed-in visitor's runs; skipping runs on the app's saved sign-ins. */
export function HostedOAuthPrompt({ oauth }: { oauth: HostedOAuth }) {
	const request = useSyncExternalStore(
		oauth.subscribe,
		oauth.pending,
		oauth.pending,
	);
	return request ? (
		<ConnectPrompt key={request.id} oauth={oauth} request={request} />
	) : null;
}
