"use client";

import type { IBoard } from "@flow-like/flow-like-ui";
import {
	type IOAuthProvider,
	type IOAuthToken,
	type IStoredOAuthToken,
	checkOAuthTokens,
} from "@flow-like/flow-like-ui";
import { useCallback, useState } from "react";
import { oauthTokenStore } from "../lib/oauth-db";
import { oauthService } from "../lib/oauth-service";

export interface OAuthExecutionState {
	/** Whether OAuth consent is pending */
	isPending: boolean;
	/** Providers that need authorization */
	missingProviders: IOAuthProvider[];
	/** Currently authorizing provider */
	authorizingProvider: string | null;
	/** Provider currently in device flow (for dialog) */
	deviceFlowProvider: IOAuthProvider | null;
}

export interface UseOAuthExecutionResult {
	/** Current state of OAuth execution */
	state: OAuthExecutionState;
	/** Check if board needs OAuth and get tokens or missing providers */
	checkBoardOAuth: (board: IBoard) => Promise<{
		tokens?: Record<string, IOAuthToken>;
		missingProviders: IOAuthProvider[];
	}>;
	/** Start OAuth flow for a provider */
	authorizeProvider: (providerId: string) => Promise<void>;
	/** Handle device flow success */
	handleDeviceFlowSuccess: (token: IStoredOAuthToken) => void;
	/** Handle device flow cancel */
	handleDeviceFlowCancel: () => void;
	/** Reset the state */
	reset: () => void;
}

/**
 * Hook to manage OAuth authorization flow for board execution.
 * Use this to check if a board requires OAuth and handle the consent flow.
 */
export function useOAuthExecution(): UseOAuthExecutionResult {
	const [state, setState] = useState<OAuthExecutionState>({
		isPending: false,
		missingProviders: [],
		authorizingProvider: null,
		deviceFlowProvider: null,
	});

	const checkBoardOAuth = useCallback(async (board: IBoard) => {
		const result = await checkOAuthTokens(board, oauthTokenStore, undefined, {
			refreshToken: oauthService.refreshToken.bind(oauthService),
		});

		if (result.missingProviders.length > 0) {
			setState({
				isPending: true,
				missingProviders: result.missingProviders,
				authorizingProvider: null,
				deviceFlowProvider: null,
			});
			return {
				tokens: undefined,
				missingProviders: result.missingProviders,
			};
		}

		setState({
			isPending: false,
			missingProviders: [],
			authorizingProvider: null,
			deviceFlowProvider: null,
		});

		return {
			tokens: Object.keys(result.tokens).length > 0 ? result.tokens : undefined,
			missingProviders: [],
		};
	}, []);

	const authorizeProvider = useCallback(
		async (providerId: string) => {
			const provider = state.missingProviders.find((p) => p.id === providerId);
			if (!provider) {
				console.error(`Provider ${providerId} not found in missing providers`);
				return;
			}

			setState((prev) => ({ ...prev, authorizingProvider: providerId }));

			// Check if provider uses device flow
			if (provider.use_device_flow && provider.device_auth_url) {
				// Open device flow dialog
				setState((prev) => ({ ...prev, deviceFlowProvider: provider }));
				return;
			}

			// Standard authorization code flow
			try {
				await oauthService.startAuthorization(provider);
			} catch (error) {
				console.error("OAuth flow error:", error);
				setState((prev) => ({ ...prev, authorizingProvider: null }));
			}
		},
		[state.missingProviders],
	);

	const handleDeviceFlowSuccess = useCallback((token: IStoredOAuthToken) => {
		setState((prev) => {
			// Remove the provider from missing list
			const newMissing = prev.missingProviders.filter(
				(p) => p.id !== token.providerId,
			);
			return {
				...prev,
				deviceFlowProvider: null,
				authorizingProvider: null,
				missingProviders: newMissing,
				isPending: newMissing.length > 0,
			};
		});
	}, []);

	const handleDeviceFlowCancel = useCallback(() => {
		setState((prev) => ({
			...prev,
			deviceFlowProvider: null,
			authorizingProvider: null,
		}));
	}, []);

	const reset = useCallback(() => {
		setState({
			isPending: false,
			missingProviders: [],
			authorizingProvider: null,
			deviceFlowProvider: null,
		});
	}, []);

	return {
		state,
		checkBoardOAuth,
		authorizeProvider,
		handleDeviceFlowSuccess,
		handleDeviceFlowCancel,
		reset,
	};
}
