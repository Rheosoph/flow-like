"use client";

import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useRef,
	useState,
} from "react";
import type { IOAuthConsentStore } from "../../db/oauth-db";
import {
	checkOAuthTokens,
	checkOAuthTokensFromPrerun,
	hasRequiredScopes,
} from "../../lib/oauth/helpers";
import type { OAuthService } from "../../lib/oauth/service";
import { getMissingOAuthScopes } from "../../lib/oauth/token-scopes";
import type {
	IOAuthProvider,
	IOAuthRuntime,
	IOAuthToken,
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "../../lib/oauth/types";
import type { IBoard } from "../../lib/schema/flow/board";
import type { IHub } from "../../lib/schema/hub/hub";
import { DeviceFlowDialog } from "./device-flow-dialog";
import { OAuthConsentDialog } from "./oauth-consent-dialog";

export interface OAuthExecutionContextValue {
	withOAuthCheck: <T>(
		board: IBoard,
		executor: (tokens?: Record<string, IOAuthToken>) => Promise<T>,
	) => Promise<T>;
	/** Check OAuth using prerun requirements (only needs execute permission, not read board) */
	withOAuthCheckFromPrerun: <T>(
		oauthRequirements: Array<{ provider_id: string; scopes: string[] }>,
		executor: (tokens?: Record<string, IOAuthToken>) => Promise<T>,
	) => Promise<T>;
	handleOAuthCallback: (providerId: string, token: IStoredOAuthToken) => void;
}

const OAuthExecutionContext = createContext<OAuthExecutionContextValue | null>(
	null,
);

export function useOAuthExecutionContext() {
	const context = useContext(OAuthExecutionContext);
	if (!context) {
		throw new Error(
			"useOAuthExecutionContext must be used within OAuthExecutionProvider",
		);
	}
	return context;
}

interface OAuthRequiredEvent extends CustomEvent {
	detail: {
		missingProviders: IOAuthProvider[];
		appId: string;
		boardId: string;
		nodeId: string;
		payload?: object;
		eventId?: string;
		sessionId?: string;
		requestId?: string;
	};
}

interface PendingExecution {
	appId: string;
	boardId: string;
	nodeId: string;
	payload?: object;
	eventId?: string;
	sessionId?: string;
	requestId?: string;
}

export interface OAuthExecutionProviderProps {
	children: ReactNode;
	oauthService: OAuthService;
	runtime: IOAuthRuntime;
	tokenStore: IOAuthTokenStoreWithPending;
	consentStore: IOAuthConsentStore;
	hub?: IHub;
	onOAuthCallback?: (providerId: string, token: IStoredOAuthToken) => void;
	providerCacheRef?: React.MutableRefObject<Map<string, IOAuthProvider>>;
}

export function OAuthExecutionProvider({
	children,
	oauthService,
	runtime,
	tokenStore,
	consentStore,
	hub,
	onOAuthCallback,
	providerCacheRef,
}: OAuthExecutionProviderProps) {
	const [missingProviders, setMissingProviders] = useState<IOAuthProvider[]>(
		[],
	);
	const [currentAppId, setCurrentAppId] = useState<string | null>(null);
	const [pendingExecution, setPendingExecution] =
		useState<PendingExecution | null>(null);
	const [isDialogOpen, setIsDialogOpen] = useState(false);
	const [authorizedProviders, setAuthorizedProviders] = useState<Set<string>>(
		new Set(),
	);
	const [missingScopesByProvider, setMissingScopesByProvider] = useState<
		Record<string, string[]>
	>({});
	const [preAuthorizedProviders, setPreAuthorizedProviders] = useState<
		Set<string>
	>(new Set());
	const [deviceFlowProvider, setDeviceFlowProvider] =
		useState<IOAuthProvider | null>(null);
	const [pendingAutoConsent, setPendingAutoConsent] = useState<
		IOAuthProvider[]
	>([]);

	const pendingExecutionRef = useRef<PendingExecution | null>(null);
	const currentAppIdRef = useRef<string | null>(null);
	const missingProvidersRef = useRef<IOAuthProvider[]>([]);

	useEffect(() => {
		pendingExecutionRef.current = pendingExecution;
	}, [pendingExecution]);

	useEffect(() => {
		currentAppIdRef.current = currentAppId;
	}, [currentAppId]);

	useEffect(() => {
		missingProvidersRef.current = missingProviders;
	}, [missingProviders]);

	// Auto-consent for providers the user has previously remembered
	useEffect(() => {
		if (pendingAutoConsent.length === 0) return;

		const processAutoConsent = async () => {
			const provider = pendingAutoConsent[0];
			const remainingProviders = pendingAutoConsent.slice(1);
			setPendingAutoConsent(remainingProviders);

			const existingToken = await tokenStore.getToken(provider.id, provider);
			const requiredScopes = provider.merged_scopes ?? provider.scopes ?? [];

			// Check if token exists, is not expired, AND has all required scopes
			const tokenHasAllScopes =
				existingToken &&
				!tokenStore.isExpired(existingToken) &&
				hasRequiredScopes(existingToken, requiredScopes);

			if (tokenHasAllScopes) {
				setAuthorizedProviders((prev) => {
					const next = new Set(prev);
					next.add(provider.id);
					return next;
				});
				return;
			}

			// Token is missing or expired or missing scopes - need to reauthorize
			if (existingToken && !tokenStore.isExpired(existingToken)) {
				console.log(
					`[OAuth] Auto-consent: Token for ${provider.id} is missing scopes. Reauthorizing...`,
				);
			}

			if (provider.use_device_flow && provider.device_auth_url) {
				setDeviceFlowProvider(provider);
			} else {
				oauthService.startAuthorization(provider);
			}
		};

		processAutoConsent();
	}, [pendingAutoConsent, tokenStore, oauthService]);

	// Populate provider cache when dialog opens
	useEffect(() => {
		if (isDialogOpen && missingProviders.length > 0 && providerCacheRef) {
			const cache = new Map<string, IOAuthProvider>();
			for (const provider of missingProviders) {
				cache.set(provider.id, provider);
			}
			providerCacheRef.current = cache;
			setAuthorizedProviders(new Set());
		}
		return () => {
			if (!isDialogOpen && providerCacheRef) {
				providerCacheRef.current = new Map();
			}
		};
	}, [isDialogOpen, missingProviders, providerCacheRef]);

	// Listen for OAuth required events
	useEffect(() => {
		const handleOAuthRequired = async (event: Event) => {
			const oauthEvent = event as OAuthRequiredEvent;
			const { missingProviders: allMissing, ...execution } = oauthEvent.detail;
			const { appId, boardId, nodeId } = execution;
			console.log("[OAuthExecutionProvider] Received oauth-required:", {
				appId,
				boardId,
				nodeId,
				missingProviders: allMissing?.length,
			});
			setCurrentAppId(appId);
			setPendingExecution(execution);

			const autoConsentProviders: IOAuthProvider[] = [];
			const needsDialogProviders: IOAuthProvider[] = [];
			const hasTokenNeedsConsent: Set<string> = new Set();
			const needsScopeUpgrade: Set<string> = new Set();

			for (const provider of allMissing) {
				const existingToken = await tokenStore.getToken(provider.id, provider);
				const requiredScopes = provider.merged_scopes ?? provider.scopes ?? [];

				// Check if token exists but is missing required scopes
				const tokenMissingScopes =
					existingToken &&
					!tokenStore.isExpired(existingToken) &&
					!hasRequiredScopes(existingToken, requiredScopes);

				// Check if consent exists but is missing required scopes (scope-aware consent check)
				const hasConsentWithRequiredScopes =
					await consentStore.hasConsentWithScopes(
						appId,
						provider.id,
						requiredScopes,
					);

				if (tokenMissingScopes) {
					// Token exists but is missing scopes - force reauthorization regardless of consent
					console.log(
						`[OAuth] Provider ${provider.id} needs scope upgrade. Required:`,
						requiredScopes,
						"Has:",
						existingToken.scopes,
					);
					needsScopeUpgrade.add(provider.id);
					needsDialogProviders.push(provider);
				} else if (hasConsentWithRequiredScopes) {
					// User previously consented AND consent covers all required scopes
					autoConsentProviders.push(provider);
				} else {
					if (existingToken && !tokenStore.isExpired(existingToken)) {
						hasTokenNeedsConsent.add(provider.id);
						needsDialogProviders.push(provider);
					} else {
						needsDialogProviders.push(provider);
					}
				}
			}

			if (autoConsentProviders.length > 0) {
				setPendingAutoConsent(autoConsentProviders);
			}

			if (needsDialogProviders.length > 0) {
				setMissingScopesByProvider({});
				setAuthorizedProviders(new Set());
				setPreAuthorizedProviders(hasTokenNeedsConsent);
				setMissingProviders(needsDialogProviders);
				setIsDialogOpen(true);
			}
		};

		window.addEventListener("flow:oauth-required", handleOAuthRequired);
		return () => {
			window.removeEventListener("flow:oauth-required", handleOAuthRequired);
		};
	}, [consentStore, tokenStore]);

	const handleAuthorize = useCallback(
		async (providerId: string) => {
			const provider = missingProvidersRef.current.find(
				(p) => p.id === providerId,
			);
			if (!provider) return;
			setMissingScopesByProvider((prev) => ({ ...prev, [providerId]: [] }));

			if (provider.use_device_flow && provider.device_auth_url) {
				setDeviceFlowProvider(provider);
				return;
			}

			await oauthService.startAuthorization(provider);
		},
		[oauthService],
	);

	const handleConfirmAll = useCallback(
		async (rememberConsent: boolean) => {
			const appId = currentAppIdRef.current;
			const providers = missingProvidersRef.current;
			const execution = pendingExecutionRef.current;

			console.log("[OAuthExecutionProvider] handleConfirmAll:", {
				appId,
				providers: providers?.length,
				execution,
				rememberConsent,
			});

			// Only persist consent if "remember" is checked
			if (rememberConsent && appId) {
				for (const provider of providers) {
					await consentStore.setConsent(
						appId,
						provider.id,
						provider.merged_scopes ?? provider.scopes,
					);
				}
				console.log(
					"[OAuthExecutionProvider] Consent saved for providers:",
					providers.map((p) => p.id),
				);
			}

			setIsDialogOpen(false);
			setMissingScopesByProvider({});
			setMissingProviders([]);
			setAuthorizedProviders(new Set());
			setPreAuthorizedProviders(new Set());

			if (execution) {
				console.log(
					"[OAuthExecutionProvider] Dispatching oauth-retry:",
					execution,
				);
				window.dispatchEvent(
					new CustomEvent("flow:oauth-retry", {
						detail: {
							...execution,
							skipConsentCheck: true,
						},
					}),
				);
				setPendingExecution(null);
			}
		},
		[consentStore],
	);

	const handleOAuthCallbackInternal = useCallback(
		(providerId: string, token: IStoredOAuthToken) => {
			const provider = missingProvidersRef.current.find(
				(p) => p.id === providerId,
			);
			if (!provider) return;
			const missingScopes = getMissingOAuthScopes(
				token,
				provider.merged_scopes ?? provider.scopes,
			);
			setMissingScopesByProvider((prev) => ({
				...prev,
				[providerId]: missingScopes,
			}));
			setAuthorizedProviders((prev) => {
				const next = new Set(prev);
				if (missingScopes.length === 0) next.add(providerId);
				else next.delete(providerId);
				return next;
			});
			if (missingScopes.length === 0) onOAuthCallback?.(providerId, token);
		},
		[onOAuthCallback],
	);

	const handleDeviceFlowSuccess = useCallback(
		(token: IStoredOAuthToken) => {
			setDeviceFlowProvider(null);
			handleOAuthCallbackInternal(token.providerId, token);
		},
		[handleOAuthCallbackInternal],
	);

	const handleDeviceFlowCancel = useCallback(() => {
		setDeviceFlowProvider(null);
	}, []);

	const handleCancel = useCallback(() => {
		const execution = pendingExecutionRef.current;
		pendingExecutionRef.current = null;
		if (execution) {
			window.dispatchEvent(
				new CustomEvent("flow:oauth-cancelled", { detail: execution }),
			);
		}
		setIsDialogOpen(false);
		setMissingScopesByProvider({});
		setMissingProviders([]);
		setAuthorizedProviders(new Set());
		setPreAuthorizedProviders(new Set());
		setPendingExecution(null);
	}, []);

	const withOAuthCheck = useCallback(
		async <T,>(
			board: IBoard,
			executor: (tokens?: Record<string, IOAuthToken>) => Promise<T>,
		): Promise<T> => {
			const result = await checkOAuthTokens(board, tokenStore, hub, {
				refreshToken: oauthService.refreshToken.bind(oauthService),
			});

			if (result.missingProviders.length === 0) {
				const tokens =
					Object.keys(result.tokens).length > 0 ? result.tokens : undefined;
				return executor(tokens);
			}

			setMissingProviders(result.missingProviders);
			setMissingScopesByProvider({});
			setIsDialogOpen(true);

			throw new Error("OAuth authorization required");
		},
		[tokenStore, hub, oauthService],
	);

	const withOAuthCheckFromPrerun = useCallback(
		async <T,>(
			oauthRequirements: Array<{ provider_id: string; scopes: string[] }>,
			executor: (tokens?: Record<string, IOAuthToken>) => Promise<T>,
		): Promise<T> => {
			const result = await checkOAuthTokensFromPrerun(
				oauthRequirements,
				tokenStore,
				hub,
				{
					refreshToken: oauthService.refreshToken.bind(oauthService),
				},
			);

			if (result.missingProviders.length === 0) {
				const tokens =
					Object.keys(result.tokens).length > 0 ? result.tokens : undefined;
				return executor(tokens);
			}

			setMissingProviders(result.missingProviders);
			setMissingScopesByProvider({});
			setIsDialogOpen(true);

			throw new Error("OAuth authorization required");
		},
		[tokenStore, hub, oauthService],
	);

	return (
		<OAuthExecutionContext.Provider
			value={{
				withOAuthCheck,
				withOAuthCheckFromPrerun,
				handleOAuthCallback: handleOAuthCallbackInternal,
			}}
		>
			{children}
			<OAuthConsentDialog
				open={isDialogOpen}
				onOpenChange={(open) => {
					if (open) setIsDialogOpen(true);
					else handleCancel();
				}}
				providers={missingProviders}
				onAuthorize={handleAuthorize}
				onConfirmAll={handleConfirmAll}
				onCancel={handleCancel}
				authorizedProviders={authorizedProviders}
				preAuthorizedProviders={preAuthorizedProviders}
				missingScopesByProvider={missingScopesByProvider}
			/>
			<DeviceFlowDialog
				provider={deviceFlowProvider}
				oauthService={oauthService}
				runtime={runtime}
				onSuccess={handleDeviceFlowSuccess}
				onCancel={handleDeviceFlowCancel}
			/>
		</OAuthExecutionContext.Provider>
	);
}
