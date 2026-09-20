"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { useAuth } from "react-oidc-context";
import { useHub } from "../../hooks/use-hub";
import { useInvoke } from "../../hooks/use-invoke";
import { isTauri } from "../../lib/platform";
import { useBackend } from "../../state/backend-state";

export function usePayments() {
	const backend = useBackend();
	const auth = useAuth();
	const { hub } = useHub();
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

export function usePaymentDistribution(): boolean {
	const policy = useQuery({
		queryKey: ["payment-distribution"],
		queryFn: async () => {
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
	});
	return policy.data === true;
}
