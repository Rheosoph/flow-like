"use client";

import {
	type QueryClient,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback } from "react";
import { useAuth } from "react-oidc-context";
import { useHub } from "../../hooks/use-hub";
import { useInvoke } from "../../hooks/use-invoke";
import { isTauri } from "../../lib/platform";
import { useBackend } from "../../state/backend-state";

export function usePayments() {
	const backend = useBackend();
	const auth = useAuth();
	const { hub, refetch: refetchConfig } = useHub();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const hubProfile = auth.isAuthenticated
		? profile.data?.hub_profile
		: undefined;
	const queryClient = useQueryClient();
	const request = useCallback(
		<T>(
			path: string,
			method: "GET" | "POST" | "PATCH" = "GET",
			body?: unknown,
		): Promise<T> => {
			if (!hubProfile) return Promise.reject(new Error("Sign in to continue."));
			if (method === "POST")
				return backend.apiState.post<T>(hubProfile, path, body);
			if (method === "PATCH")
				return backend.apiState.patch<T>(hubProfile, path, body);
			return backend.apiState.get<T>(hubProfile, path);
		},
		[backend.apiState, hubProfile],
	);
	const refresh = useCallback(
		() => queryClient.invalidateQueries({ queryKey: ["payments"] }),
		[queryClient],
	);
	return {
		request,
		refresh,
		ready: !!hubProfile,
		identity: [hubProfile?.hub, auth.user?.profile.sub],
		config: hub?.payments,
		/** False until the hub answered, when `config` can't tell "off" from "unknown". */
		configLoaded: hub !== undefined,
		refetchConfig,
		auth,
	};
}

export function usePaymentQuery<T>(path: string, enabled = true, poll = false) {
	const payments = usePayments();
	return useQuery({
		queryKey: ["payments", ...payments.identity, path],
		queryFn: () => payments.request<T>(path),
		enabled: payments.ready && enabled,
		refetchInterval: poll ? 3000 : false,
		refetchOnWindowFocus: true,
		retry: false,
		gcTime: 0,
		meta: { persist: false },
	});
}

/** Whether this build may sell anything. Store-distributed desktop builds may not. */
const paymentDistributionQuery = {
	queryKey: ["payment-distribution"],
	queryFn: async (): Promise<boolean> => {
		if (!isTauri()) return true;
		const { invoke } = await import("@tauri-apps/api/core");
		const capabilities = await invoke<{ payments_allowed?: boolean }>(
			"get_system_info",
		);
		return capabilities.payments_allowed === true;
	},
	staleTime: Number.POSITIVE_INFINITY,
	retry: false,
	meta: { persist: false },
};

export function usePaymentDistribution(): boolean {
	const policy = useQuery(paymentDistributionQuery);
	return policy.data === true;
}

/** {@link usePaymentDistribution} for code that runs outside a component. */
export function purchasingAllowed(queryClient: QueryClient): Promise<boolean> {
	return queryClient
		.ensureQueryData(paymentDistributionQuery)
		.catch(() => false);
}
