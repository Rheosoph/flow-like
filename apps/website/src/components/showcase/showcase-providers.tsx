import { createIDBPersister } from "@flow-like/flow-like-ui/lib/persister";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import { useBackendStore } from "@flow-like/flow-like-ui/state/backend-state";
import { QueryClient } from "@tanstack/react-query";
import { PersistQueryClientProvider } from "@tanstack/react-query-persist-client";
import { type ReactNode, Suspense, useEffect, useState } from "react";

/**
 * Single source of the react-query client + IDB persister for every showcase
 * island (chat, board, runs, catalog). `board-wrapper.tsx` consumes these too
 * so the setup is never duplicated.
 */
export const showcaseQueryClient = new QueryClient({
	defaultOptions: {
		queries: {
			networkMode: "offlineFirst",
			staleTime: 1000,
			gcTime: 24 * 60 * 60 * 1000,
			refetchOnWindowFocus: false,
			refetchOnReconnect: "always",
			refetchOnMount: false,
			retry: 1,
			retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
		},
	},
});

export const showcasePersister = createIDBPersister();

function BackendSetter({
	backend,
	children,
}: Readonly<{ backend: IBackendState; children: ReactNode }>) {
	const { setBackend } = useBackendStore();
	const [ready, setReady] = useState(false);

	useEffect(() => {
		setBackend(backend);
		setReady(true);
	}, [backend, setBackend]);

	if (!ready) return null;
	return <>{children}</>;
}

/**
 * Wraps a showcase island in — on demand — the react-query provider and a stub
 * backend. Dark theming is scoped via a `.dark` ancestor on the Astro wrapper
 * (LiveShowcase.astro), NOT next-themes, so islands never fight over the global
 * `documentElement` theme. Chat needs neither `query` nor `backend`;
 * board/runs/catalog need both.
 */
export function ShowcaseProviders({
	children,
	query = false,
	backend,
}: Readonly<{
	children: ReactNode;
	query?: boolean;
	backend?: IBackendState;
}>) {
	const tree = backend ? (
		<BackendSetter backend={backend}>{children}</BackendSetter>
	) : (
		children
	);

	if (query) {
		return (
			<Suspense fallback={null}>
				<PersistQueryClientProvider
					client={showcaseQueryClient}
					persistOptions={{ persister: showcasePersister }}
				>
					{tree}
				</PersistQueryClientProvider>
			</Suspense>
		);
	}

	return <>{tree}</>;
}
