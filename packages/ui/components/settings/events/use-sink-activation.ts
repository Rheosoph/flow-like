"use client";

import { useCallback, useEffect, useState } from "react";
import type { IOAuthConsentStore } from "../../../db/oauth-db";
import {
	checkOAuthTokens,
	checkOAuthTokensFromPrerun,
} from "../../../lib/oauth/helpers";
import type {
	IOAuthProvider,
	IOAuthToken,
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "../../../lib/oauth/types";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IHub } from "../../../lib/schema/hub/hub";
import { useBackend } from "../../../state/backend-state";

export interface SinkActivationOptions {
	appId: string;
	tokenStore?: IOAuthTokenStoreWithPending;
	consentStore?: IOAuthConsentStore;
	hub?: IHub;
	onStartOAuth?: (provider: IOAuthProvider) => Promise<void>;
	onRefreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
	/** Called after a successful write, so the caller can re-read sink state. */
	onChanged?: (event: IEvent) => void | Promise<void>;
}

interface ToggleTarget {
	event: IEvent;
	active: boolean;
}

export interface RequestToggleOptions {
	/** Desired state after the toggle. */
	active: boolean;
	/** Whether this event type registers a sink, which is what needs authorizing. */
	requiresSink: boolean;
}

/**
 * Switching an event on or off is four flows wearing one button: a plain event
 * writes straight through, a sink on an offline project activates locally, and
 * a sink on an online project needs a PAT — preceded, when switching on, by
 * OAuth tokens and per-app consent. Turning a sink off skips the OAuth leg
 * (there is nothing left to authorize) but still needs the PAT, because the
 * server is being asked to tear a registration down.
 */
export function useSinkActivation({
	appId,
	tokenStore,
	consentStore,
	hub,
	onStartOAuth,
	onRefreshToken,
	onChanged,
}: SinkActivationOptions) {
	const backend = useBackend();
	const [target, setTarget] = useState<ToggleTarget | null>(null);
	const [pendingId, setPendingId] = useState<string | null>(null);
	const [showPatDialog, setShowPatDialog] = useState(false);
	const [showConsentDialog, setShowConsentDialog] = useState(false);
	const [missingProviders, setMissingProviders] = useState<IOAuthProvider[]>(
		[],
	);
	const [authorizedProviders, setAuthorizedProviders] = useState<Set<string>>(
		new Set(),
	);
	const [preAuthorizedProviders, setPreAuthorizedProviders] = useState<
		Set<string>
	>(new Set());
	const [pendingTokens, setPendingTokens] = useState<
		Record<string, IOAuthToken>
	>({});

	const reset = useCallback(() => {
		setShowPatDialog(false);
		setShowConsentDialog(false);
		setMissingProviders([]);
		setAuthorizedProviders(new Set());
		setPreAuthorizedProviders(new Set());
		setPendingTokens({});
		setTarget(null);
		setPendingId(null);
	}, []);

	const write = useCallback(
		async (
			next: ToggleTarget,
			patOrTokens?: string | Record<string, IOAuthToken>,
		) => {
			const pat = typeof patOrTokens === "string" ? patOrTokens : undefined;
			const tokens = typeof patOrTokens === "object" ? patOrTokens : undefined;
			setPendingId(next.event.id);
			try {
				await backend.eventState.upsertEvent(
					appId,
					{ ...next.event, active: next.active },
					undefined,
					pat,
					tokens,
				);
				await onChanged?.(next.event);
			} catch (error) {
				console.error(
					`Failed to set event ${next.event.id} active=${next.active}:`,
					error,
				);
			} finally {
				reset();
			}
		},
		[appId, backend.eventState, onChanged, reset],
	);

	const requestToggle = useCallback(
		async (event: IEvent, { active, requiresSink }: RequestToggleOptions) => {
			const next: ToggleTarget = { event, active };
			setTarget(next);

			if (!requiresSink) {
				await write(next);
				return;
			}

			const offline = await backend.isOffline(appId);
			if (offline) {
				await write(next);
				return;
			}

			if (active && tokenStore) {
				try {
					let result: Awaited<ReturnType<typeof checkOAuthTokens>> | undefined;
					const version = event.board_version as
						| [number, number, number]
						| undefined;

					// Execute-only members cannot read the board, so fall back to the
					// prerun endpoint, which reports requirements without it.
					const board = await backend.boardState
						.getBoard(appId, event.board_id, version)
						.catch(() => undefined);

					if (board) {
						result = await checkOAuthTokens(board, tokenStore, hub, {
							refreshToken: onRefreshToken,
						});
					} else if (backend.eventState.prerunEvent) {
						const prerun = await backend.eventState.prerunEvent(
							appId,
							event.id,
							version,
						);
						result = await checkOAuthTokensFromPrerun(
							prerun.oauth_requirements,
							tokenStore,
							hub,
							{ refreshToken: onRefreshToken },
						);
					}

					if (result && result.requiredProviders.length > 0) {
						const consented = consentStore
							? await consentStore.getConsentedProviderIds(appId)
							: new Set<string>();
						const needConsent: IOAuthProvider[] = [...result.missingProviders];
						const haveTokenNeedConsent = new Set<string>();

						for (const provider of result.requiredProviders) {
							const hasToken = result.tokens[provider.id] !== undefined;
							if (hasToken && !consented.has(provider.id)) {
								haveTokenNeedConsent.add(provider.id);
								needConsent.push(provider);
							}
						}

						if (needConsent.length > 0) {
							setPendingTokens(result.tokens);
							setMissingProviders(needConsent);
							setPreAuthorizedProviders(haveTokenNeedConsent);
							setAuthorizedProviders(new Set());
							setShowConsentDialog(true);
							return;
						}

						if (Object.keys(result.tokens).length > 0) {
							await write(next, result.tokens);
							return;
						}
					}
				} catch (error) {
					console.error("Failed to check OAuth requirements:", error);
				}
			}

			setShowPatDialog(true);
		},
		[appId, backend, consentStore, hub, onRefreshToken, tokenStore, write],
	);

	const authorizeProvider = useCallback(
		async (providerId: string) => {
			const provider = missingProviders.find((p) => p.id === providerId);
			if (!provider || !onStartOAuth) return;
			await onStartOAuth(provider);
		},
		[missingProviders, onStartOAuth],
	);

	const confirmConsent = useCallback(
		async (remember: boolean) => {
			if (!target) return;
			if (remember && consentStore) {
				for (const provider of missingProviders) {
					await consentStore.setConsent(appId, provider.id, provider.scopes);
				}
			}
			setShowConsentDialog(false);

			const tokens = { ...pendingTokens };
			for (const providerId of authorizedProviders) {
				const token = await tokenStore?.getToken(providerId);
				if (token && !tokenStore?.isExpired(token)) {
					tokens[providerId] = {
						access_token: token.access_token,
						refresh_token: token.refresh_token,
						expires_at: token.expires_at
							? Math.floor(token.expires_at / 1000)
							: undefined,
						token_type: token.token_type ?? "Bearer",
					};
				}
			}

			if (Object.keys(tokens).length > 0) {
				await write(target, tokens);
				return;
			}
			setShowPatDialog(true);
		},
		[
			appId,
			authorizedProviders,
			consentStore,
			missingProviders,
			pendingTokens,
			target,
			tokenStore,
			write,
		],
	);

	const selectPat = useCallback(
		async (pat: string) => {
			if (!target) return;
			await write(target, pat);
		},
		[target, write],
	);

	// The authorization happens in a browser window, so the only way to learn it
	// finished is to watch the token store while the dialog is open.
	useEffect(() => {
		if (!showConsentDialog || !tokenStore || missingProviders.length === 0) {
			return;
		}

		const poll = async () => {
			const authorized = new Set(authorizedProviders);
			const tokens = { ...pendingTokens };

			for (const provider of missingProviders) {
				if (
					authorized.has(provider.id) ||
					preAuthorizedProviders.has(provider.id)
				) {
					continue;
				}
				const token = await tokenStore.getToken(provider.id);
				if (token && !tokenStore.isExpired(token)) {
					authorized.add(provider.id);
					tokens[provider.id] = {
						access_token: token.access_token,
						refresh_token: token.refresh_token,
						expires_at: token.expires_at
							? Math.floor(token.expires_at / 1000)
							: undefined,
						token_type: token.token_type ?? "Bearer",
					};
				}
			}

			if (authorized.size !== authorizedProviders.size) {
				setAuthorizedProviders(authorized);
				setPendingTokens(tokens);
			}
		};

		poll();
		const interval = setInterval(poll, 1000);
		return () => clearInterval(interval);
	}, [
		showConsentDialog,
		tokenStore,
		missingProviders,
		authorizedProviders,
		preAuthorizedProviders,
		pendingTokens,
	]);

	return {
		requestToggle,
		/** Id of the event currently being written, for a per-row spinner. */
		pendingId,
		dialogProps: {
			pat: {
				open: showPatDialog,
				onOpenChange: (open: boolean) => {
					if (!open) reset();
				},
				onPatSelected: selectPat,
			},
			consent: {
				open: showConsentDialog,
				onOpenChange: (open: boolean) => {
					if (!open) reset();
				},
				providers: missingProviders,
				authorizedProviders,
				preAuthorizedProviders,
				onAuthorize: authorizeProvider,
				onConfirmAll: confirmConsent,
				onCancel: reset,
			},
		},
	};
}
