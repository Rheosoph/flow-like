import {
	type UseQueryResult,
	useBackend,
	useInvoke,
	useQuery,
} from "@flow-like/flow-like-ui";

export function useApi<T>(
	method: "GET" | "POST" | "PUT" | "DELETE" | "PATCH",
	path: string,
	data?: unknown,
	enabled?: boolean,
): UseQueryResult<T, Error> {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const query = useQuery<T, Error>({
		queryKey: [method, path, data],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.fetch<T>(profile.data, path, {
				method,
				body: data ? JSON.stringify(data) : undefined,
			});
		},
		enabled: enabled && !!profile.data,
	});

	return query;
}
