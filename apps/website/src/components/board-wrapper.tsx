import {
	LoadingScreen,
	QueryClient,
	createIDBPersister,
} from "@flow-like/flow-like-ui";
import { Suspense, lazy } from "react";

const PersistQueryClientProvider = lazy(() =>
	import("@flow-like/flow-like-ui").then((module) => ({
		default: module.PersistQueryClientProvider,
	})),
);

const Board = lazy(() => import("./board"));
const persister = createIDBPersister();
const queryClient = new QueryClient({
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
export default function BoardWrapper({
	nodes,
	edges,
}: Readonly<{ nodes: any[]; edges: any[] }>) {
	return (
		<Suspense fallback={<LoadingScreen />}>
			<PersistQueryClientProvider
				client={queryClient}
				persistOptions={{
					persister,
				}}
			>
				<div className="w-full h-full">
					<Board nodes={nodes} edges={edges} />
				</div>
			</PersistQueryClientProvider>
		</Suspense>
	);
}
