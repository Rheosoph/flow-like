"use client";
import {
	type QueryClient,
	type UseQueryResult,
	useInfiniteQuery,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { isEqual } from "lodash-es";
import { useBackend } from "../state/backend-state";

type BackendFunction<T, Args extends any[]> = (...args: Args) => Promise<T>;

/**
 * Custom hook to invoke an asynchronous backend function using React Query.
 * Handles functions with multiple arguments passed as an array.
 *
 * @template T The expected return type of the backend function.
 * @template Args An array type representing the arguments of the backend function.
 * @param {BackendFunction<T, Args>} backendFn The asynchronous function to call.
 * @param {Args} args An array containing the arguments to pass to the backend function.
 * @param {boolean} [enabled=true] Whether the query should be enabled and run automatically.
 * Defaults to true.
 * @param {any[]} [additionalDeps=[]] Optional additional dependencies to include in the queryKey,
 * beyond the function name and arguments. Useful if the query
 * depends on external factors not passed as arguments.
 * @returns {UseQueryResult<T, Error>} The result object from React Query, containing data, error, status, etc.
 */
export function useInvoke<T, Args extends any[]>(
	backendFn: BackendFunction<T, Args>,
	backendContext: any,
	args: Args,
	enabled = true,
	additionalDeps: any[] = [],
	staleTime?: number,
): UseQueryResult<T, Error> {
	const backend = useBackend();
	const query = useQuery<T, Error>({
		queryKey: [
			backendFn.name || "backendFn",
			...args,
			...additionalDeps,
		].filter((arg) => typeof arg !== "undefined"),
		queryFn: async () => {
			try {
				const boundFn = backendFn.bind(backendContext ?? backend);
				const response = await boundFn(...args);
				return response; // No need to cast if types are correctly inferred/set
			} catch (error) {
				console.error("Error invoking backend function:", error);
				if (error instanceof Error) {
					throw error;
				}
				throw new Error(String(error));
			}
		},
		enabled,
		staleTime,
		// Use defaults from QueryClient, but allow individual overrides if needed
		// This ensures consistency with global cache settings
	});

	return query;
}

type PaginationParams = {
	offset: number;
	limit: number;
};

type BackendFunctionWithPagination<T, Args extends any[]> = (
	...args: [...Args, number, number]
) => Promise<T>;

/**
 * Custom hook to invoke an asynchronous backend function with infinite scrolling using React Query.
 * The backend function must accept offset and limit parameters as the last argument.
 *
 * @template T The expected return type of the backend function.
 * @template Args An array type representing the arguments of the backend function (excluding pagination params).
 * @param {BackendFunctionWithPagination<T, Args>} backendFn The asynchronous function to call with pagination.
 * @param {Args} args An array containing the arguments to pass to the backend function (excluding pagination).
 * @param {number} [pageSize=20] The number of items to fetch per page (limit parameter).
 * @param {boolean} [enabled=true] Whether the query should be enabled and run automatically.
 * @param {any[]} [additionalDeps=[]] Optional additional dependencies to include in the queryKey.
 * @param {number} [staleTime] Optional staleTime override for this query.
 * @returns {UseInfiniteQueryResult<T, Error>} The result object from React Query infinite query.
 */
export function useInfiniteInvoke<T, Args extends any[]>(
	backendFn: BackendFunctionWithPagination<T, Args>,
	backendContext: any,
	args: Args,
	pageSize = 50,
	enabled = true,
	additionalDeps: any[] = [],
	staleTime?: number,
) {
	const backend = useBackend();

	const query = useInfiniteQuery<T, Error>({
		queryKey: [
			backendFn.name || "infiniteBackendFn",
			...args,
			pageSize,
			...additionalDeps,
		].filter((arg) => typeof arg !== "undefined"),
		queryFn: async ({ pageParam = 0 }) => {
			try {
				const boundFn = backendFn.bind(backendContext ?? backend);
				const paginationParams: PaginationParams = {
					offset: pageParam as number,
					limit: pageSize,
				};
				// Destructure pagination params and pass as separate arguments
				const response = await boundFn(
					...args,
					paginationParams.offset,
					paginationParams.limit,
				);
				return response;
			} catch (error) {
				console.error("Error invoking infinite backend function:", error);
				if (error instanceof Error) {
					throw error;
				}
				throw new Error(String(error));
			}
		},
		getNextPageParam: (lastPage, allPages) => {
			// Assuming the response has a way to determine if there are more pages
			// You might need to adjust this based on your API response structure
			const currentOffset = (allPages.length - 1) * pageSize;
			const nextOffset = currentOffset + pageSize;

			// If lastPage is an array and has more than 0 items, we've reached the end
			if (Array.isArray(lastPage) && lastPage.length === 0) {
				return undefined;
			}

			// If lastPage has a property indicating more data, use that
			if (
				typeof lastPage === "object" &&
				lastPage !== null &&
				"hasMore" in lastPage
			) {
				return (lastPage as any).hasMore ? nextOffset : undefined;
			}

			// Default: return next offset (you may want to customize this logic)
			return nextOffset;
		},
		initialPageParam: 0,
		enabled,
		...(staleTime !== undefined && { staleTime }),
	});

	return query;
}

/**
 * Custom hook that returns a function to invalidate React Query cache entries
 * associated with a specific backend function used via `useInfiniteInvoke`.
 * This typically invalidates all infinite queries starting with the function's name key prefix.
 *
 * @returns {function(backendFn: BackendFunctionWithPagination<any, any[]>): Promise<void>}
 * A function that accepts the backend function whose infinite queries should be invalidated.
 */
export function useInvalidateInfiniteInvoke() {
	const queryClient = useQueryClient();

	/**
	 * Invalidates infinite queries associated with the given backend function.
	 * Uses prefix matching based on the function name.
	 *
	 * @template T The return type of the backend function (often not needed for invalidation).
	 * @template Args The arguments array type of the backend function (often not needed for invalidation).
	 * @param {BackendFunctionWithPagination<T, Args>} backendFn The backend function used in `useInfiniteInvoke` calls.
	 * @param {Args} args An array containing the arguments to pass to the backend function (excluding pagination).
	 * @param {number} [pageSize=50] The page size used in the original query.
	 * @param {any[]} [additionalDeps=[]] Optional additional dependencies to include in the queryKey.
	 * @returns {Promise<void>} A promise that resolves when the invalidation is complete.
	 */
	const invalidate = <T, Args extends any[]>(
		backendFn: BackendFunctionWithPagination<T, Args>,
		args: Args,
		pageSize = 50,
		additionalDeps: any[] = [],
	): Promise<void> => {
		const queryKeyPrefix = [
			backendFn.name || "infiniteBackendFn",
			...args,
			pageSize,
			...additionalDeps,
		].filter((arg) => typeof arg !== "undefined");
		return queryClient.invalidateQueries({
			queryKey: queryKeyPrefix,
		});
	};

	return invalidate;
}

/**
 * Custom hook that returns a function to invalidate React Query cache entries
 * associated with a specific backend function used via `useInvoke`.
 * This typically invalidates all queries starting with the function's name key prefix.
 *
 * @returns {function(backendFn: BackendFunction<any, any[]>): Promise<void>}
 * A function that accepts the backend function whose queries should be invalidated.
 */
export function useInvalidateInvoke() {
	const queryClient = useQueryClient();

	/**
	 * Invalidates queries associated with the given backend function.
	 * Uses prefix matching based on the function name.
	 *
	 * @template T The return type of the backend function (often not needed for invalidation).
	 * @template Args The arguments array type of the backend function (often not needed for invalidation).
	 * @param {BackendFunction<T, Args>} backendFn The backend function used in `useInvoke` calls.
	 * @returns {Promise<void>} A promise that resolves when the invalidation is complete.
	 */
	const invalidate = <T, Args extends any[]>(
		backendFn: BackendFunction<T, Args>,
		args: Args,
		additionalDeps: any[] = [],
	): Promise<void> => {
		const queryKeyPrefix = [
			backendFn.name || "backendFn",
			...args,
			...additionalDeps,
		].filter((arg) => typeof arg !== "undefined");
		return queryClient.invalidateQueries({
			queryKey: queryKeyPrefix,
		});
	};

	return invalidate;
}

export function injectData<T, Args extends any[]>(
	queryClient: QueryClient,
	backendFn: BackendFunction<T, Args>,
	args: Args,
	data: T,
	additionalDeps: any[] = [],
): UseQueryResult<T, Error> {
	const queryKey = [
		backendFn.name || "backendFn",
		...args,
		...additionalDeps,
	].filter((arg) => typeof arg !== "undefined");

	// Immediately set the data in the cache
	queryClient.setQueryData(queryKey, data);

	// Return a query object that reflects the injected data
	return {
		data,
		error: null,
		isLoading: false,
		isError: false,
		isSuccess: true,
		status: "success",
		refetch: () => Promise.resolve({ data, error: null }),
	} as UseQueryResult<T, Error>;
}

export async function injectDataFunction<T, Args extends any[]>(
	lambda: () => Promise<T>,
	context: any,
	queryClient: QueryClient,
	backendFn: BackendFunction<T, Args>,
	args: Args,
	additionalDeps: any[] = [],
	oldData?: T,
): Promise<UseQueryResult<T, Error>> {
	try {
		const boundLambda = lambda.bind(context);
		const result = await boundLambda();
		const queryKey = [
			backendFn.name || "backendFn",
			...args,
			...additionalDeps,
		].filter((arg) => typeof arg !== "undefined");

		if (!isEqual(result, oldData)) {
			queryClient?.setQueryData(queryKey, result);
		}

		return {
			data: result,
			error: null,
			isLoading: false,
			isError: false,
			isSuccess: true,
			status: "success",
			refetch: () => Promise.resolve({ data: result, error: null }),
		} as UseQueryResult<T, Error>;
	} catch (error) {
		console.error("Error invoking lambda function:", error);
		if (error instanceof Error) {
			throw error;
		}
		return {
			data: oldData as T,
			error: error as Error,
			isLoading: false,
			isError: true,
			isSuccess: false,
			status: "error",
			refetch: () =>
				Promise.resolve({ data: oldData as T, error: error as Error }),
		} as UseQueryResult<T, Error>;
	}
}

export function invalidateData<T, Args extends any[]>(
	queryClient: QueryClient,
	backendFn: BackendFunction<T, Args>,
	args: Args,
	additionalDeps: any[] = [],
): void {
	const queryKey = [
		backendFn.name || "backendFn",
		...args,
		...additionalDeps,
	].filter((arg) => typeof arg !== "undefined");
	queryClient.invalidateQueries({ queryKey });
}
