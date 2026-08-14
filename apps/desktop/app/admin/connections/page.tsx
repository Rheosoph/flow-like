"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type IProcessGraphResponse,
	ProcessGraph,
	useBackend,
	useInvoke,
	useQuery,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import { Waypoints } from "lucide-react";
import { useCallback, useState } from "react";

export default function AdminConnectionsPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [days, setDays] = useState(30);

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const graph = useQuery<IProcessGraphResponse>({
		queryKey: ["admin", "connections", "graph", days],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<IProcessGraphResponse>(
				profile.data,
				`admin/connections/graph?days=${days}`,
			);
		},
		enabled: !!profile.data,
	});

	const invalidateGraph = useCallback(async () => {
		await queryClient.invalidateQueries({
			queryKey: ["admin", "connections", "graph"],
		});
	}, [queryClient]);

	const handleRefresh = useCallback(() => {
		graph.refetch();
	}, [graph]);

	const handleCreateNote = useCallback(
		async (targetAppId: string, content: string) => {
			await backend.teamState.createProcessNote(targetAppId, content);
			await invalidateGraph();
		},
		[backend, invalidateGraph],
	);

	const handleUpdateNote = useCallback(
		async (targetAppId: string, noteId: string, content: string) => {
			await backend.teamState.updateProcessNote(targetAppId, noteId, content);
			await invalidateGraph();
		},
		[backend, invalidateGraph],
	);

	const handleDeleteNote = useCallback(
		async (targetAppId: string, noteId: string) => {
			await backend.teamState.deleteProcessNote(targetAppId, noteId);
			await invalidateGraph();
		},
		[backend, invalidateGraph],
	);

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-6xl space-y-6">
					<div>
						<h1 className="text-3xl font-bold flex items-center gap-2">
							<Waypoints className="h-7 w-7" />
							{t('processGraph', 'Process Graph')}
						</h1>
						<p className="text-muted-foreground">
							{t('platformwideViewOfAppConnectionsObservedCallChainsAndProcessNotes', "Platform-wide view of app connections, observed call chains, and process notes")}
						</p>
					</div>

					<ProcessGraph
						data={graph.data}
						isLoading={graph.isFetching}
						days={days}
						onDaysChange={setDays}
						onRefresh={handleRefresh}
						onCreateNote={handleCreateNote}
						onUpdateNote={handleUpdateNote}
						onDeleteNote={handleDeleteNote}
					/>
				</div>
			</div>
		</main>
	);
}
