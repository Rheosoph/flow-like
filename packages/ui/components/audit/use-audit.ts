"use client";

import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import type { IProfile } from "../../lib/schema/profile/profile";
import { useBackend } from "../../state/backend-state";
import type {
	IAuditChainReport,
	IAuditHead,
	IAuditHeadCheckRequest,
	IAuditHeadCheckResponse,
	IAuditRecordCursor,
	IAuditRecordFilters,
	IAuditRecordPage,
} from "./types";

export const AUDIT_PAGE_SIZE = 50;

/** Audit rows carry IPs and details that expire; they never outlive the session. */
const LIVE_ONLY = { persist: false } as const;

function recordsPath(
	chainId: string,
	filters: IAuditRecordFilters,
	cursor: IAuditRecordCursor | null,
	limit: number,
): string {
	const params = new URLSearchParams({ chain_id: chainId, limit: `${limit}` });
	for (const [key, value] of Object.entries(filters)) {
		const trimmed = value?.trim();
		if (trimmed) params.set(key, trimmed);
	}
	if (cursor) {
		params.set("before_ms", `${cursor.before_ms}`);
		params.set("before_id", cursor.before_id);
	}
	return `audit/records?${params.toString()}`;
}

export function useAuditRecords(
	profile: IProfile | undefined,
	chainId: string,
	filters: IAuditRecordFilters = {},
	limit = AUDIT_PAGE_SIZE,
) {
	const backend = useBackend();
	return useInfiniteQuery({
		queryKey: ["audit", "records", profile?.hub, chainId, filters, limit],
		initialPageParam: null as IAuditRecordCursor | null,
		queryFn: ({ pageParam }) => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<IAuditRecordPage>(
				profile,
				recordsPath(chainId, filters, pageParam, limit),
			);
		},
		getNextPageParam: (last) => last?.next ?? undefined,
		enabled: !!profile && chainId.length > 0,
		meta: LIVE_ONLY,
	});
}

/** Runs only on demand: verification reads every new seal of the chain. */
export function useAuditVerify(
	profile: IProfile | undefined,
	chainId: string,
	full = false,
) {
	const backend = useBackend();
	return useQuery({
		queryKey: ["audit", "verify", profile?.hub, chainId, full],
		queryFn: () => {
			if (!profile) throw new Error("Profile not loaded");
			const params = new URLSearchParams({
				chain_id: chainId,
				full: `${full}`,
			});
			return backend.apiState.get<IAuditChainReport>(
				profile,
				`audit/verify?${params.toString()}`,
			);
		},
		enabled: false,
		retry: false,
		meta: LIVE_ONLY,
	});
}

export function useAuditHead(profile: IProfile | undefined, chainId: string) {
	const backend = useBackend();
	return useQuery({
		queryKey: ["audit", "head", profile?.hub, chainId],
		queryFn: () => {
			if (!profile) throw new Error("Profile not loaded");
			const params = new URLSearchParams({ chain_id: chainId });
			return backend.apiState.get<IAuditHead>(
				profile,
				`audit/head?${params.toString()}`,
			);
		},
		enabled: !!profile && chainId.length > 0,
		meta: LIVE_ONLY,
	});
}

export function useAuditHeadCheck(profile: IProfile | undefined) {
	const backend = useBackend();
	return useMutation({
		mutationFn: (head: IAuditHeadCheckRequest) => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.post<IAuditHeadCheckResponse>(
				profile,
				"audit/head/check",
				head,
			);
		},
	});
}
