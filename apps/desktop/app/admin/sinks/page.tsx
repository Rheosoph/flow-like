"use client";

import {
	type IListTokensResponse,
	type IRegisterSinkRequest,
	type IRegisterSinkResponse,
	type IRevokeSinkResponse,
	type ServiceSinkType,
	SinkTokensPage,
	useBackend,
	useInvoke,
	useQuery,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";

export default function AdminSinksPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const queryClient = useQueryClient();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [sinkTypeFilter, setSinkTypeFilter] = useState<
		ServiceSinkType | undefined
	>(undefined);
	const [includeRevoked, setIncludeRevoked] = useState(false);

	const queryParams = useMemo(() => {
		const params: Record<string, string | boolean> = {};
		if (sinkTypeFilter) params.sink_type = sinkTypeFilter;
		if (includeRevoked) params.include_revoked = true;
		return params;
	}, [sinkTypeFilter, includeRevoked]);

	const tokens = useQuery<IListTokensResponse, Error>({
		queryKey: ["admin", "sinks", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const queryString = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			const path = queryString ? `admin/sinks?${queryString}` : "admin/sinks";
			return backend.apiState.get<IListTokensResponse>(profile.data, path);
		},
		enabled: !!profile.data,
	});

	const handleRefresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "sinks"],
		});
	}, [queryClient]);

	const handleRegisterToken = useCallback(
		async (request: IRegisterSinkRequest): Promise<IRegisterSinkResponse> => {
			if (!profile.data) throw new Error("Profile not loaded");

			try {
				const response = await backend.apiState.post<IRegisterSinkResponse>(
					profile.data,
					"admin/sinks",
					request,
				);
				toast.success("Token created successfully");
				return response;
			} catch (error) {
				const message =
					error instanceof Error
						? error.message
						: t("unknownError", "Unknown error");
				toast.error(`Failed to create token: ${message}`);
				throw error;
			}
		},
		[profile.data, backend.apiState],
	);

	const handleRevokeToken = useCallback(
		async (jti: string): Promise<IRevokeSinkResponse> => {
			if (!profile.data) throw new Error("Profile not loaded");

			try {
				const response = await backend.apiState.del<IRevokeSinkResponse>(
					profile.data,
					`admin/sinks/${jti}`,
				);
				toast.success("Token revoked successfully");
				return response;
			} catch (error) {
				const message =
					error instanceof Error
						? error.message
						: t("unknownError", "Unknown error");
				toast.error(`Failed to revoke token: ${message}`);
				throw error;
			}
		},
		[profile.data, backend.apiState],
	);

	return (
		<main className="flex grow h-full bg-background max-h-full overflow-auto flex-col items-start w-full justify-start p-6">
			<SinkTokensPage
				data={tokens.data}
				isLoading={tokens.isLoading}
				error={tokens.error}
				sinkTypeFilter={sinkTypeFilter}
				includeRevoked={includeRevoked}
				onSinkTypeFilterChange={setSinkTypeFilter}
				onIncludeRevokedChange={setIncludeRevoked}
				onRefresh={handleRefresh}
				onRegisterToken={handleRegisterToken}
				onRevokeToken={handleRevokeToken}
			/>
		</main>
	);
}
