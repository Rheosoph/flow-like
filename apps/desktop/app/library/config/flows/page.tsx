"use client";
import {
	type FlowLibraryBoardCreationState,
	FlowsOverviewPage,
	IExecutionStage,
	ILogLevel,
	PermissionGate,
	RolePermissions,
	useAppPermissions,
	useBackend,
	useFlowBoardParentState,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { invoke } from "@tauri-apps/api/core";
import { useRouter, useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { toast } from "sonner";

export default function Page() {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const addBoardParents = useFlowBoardParentState(
		(state) => state.addBoardParents,
	);
	const searchParams = useSearchParams();
	const id = searchParams.get("id");
	const permissions = useAppPermissions(id);
	const canReadBoards = permissions.can(RolePermissions.ReadBoards);
	const canWriteBoards = permissions.can(RolePermissions.WriteBoards);
	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[id ?? ""],
		typeof id === "string",
	);
	const boards = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[id ?? "", ["metrics", "node_types"]],
		typeof id === "string" && canReadBoards,
	);

	useEffect(() => {
		if (!app.data) return;
		if (!boards.data) return;
		const parentLink = `/library/config/flows?id=${id}`;
		addBoardParents(
			Object.fromEntries(boards.data.map((board) => [board.id, parentLink])),
		);
	}, [app.data, boards.data, id, addBoardParents]);

	const router = useRouter();
	const [boardCreation, setBoardCreation] =
		useState<FlowLibraryBoardCreationState>({
			open: false,
			name: "",
			description: "",
		});

	const handleCreateBoard = async () => {
		if (!id) return;
		if (!canWriteBoards) {
			toast.error(
				t(
					"yourRoleCannotCreateFlowsInThisProject",
					"Your role cannot create flows in this project.",
				),
			);
			return;
		}
		await backend.boardState.upsertBoard(
			id,
			createId(),
			boardCreation.name,
			boardCreation.description,
			ILogLevel.Debug,
			IExecutionStage.Dev,
		);
		await Promise.allSettled([await boards.refetch(), await app.refetch()]);
		setBoardCreation({
			name: "",
			description: "",
			open: false,
		});
	};

	const handleOpenBoard = async (boardId: string) => {
		if (!app.data) return;
		// Only a warm-up: a board listed from the hub may not be on disk yet, and
		// the editor loads it (local first, then remote) on its own.
		await invoke("get_app_board", {
			appId: app.data.id,
			boardId,
			pushToRegistry: true,
		}).catch((error) => {
			console.warn(`[Flows] Preloading board ${boardId} failed:`, error);
		});
		router.push(`/flow?id=${boardId}&app=${app.data.id}`);
	};

	const handleDeleteBoard = async (boardId: string) => {
		if (!app.data) return;
		if (!canWriteBoards) {
			toast.error(
				t(
					"yourRoleCannotDeleteFlowsInThisProject",
					"Your role cannot delete flows in this project.",
				),
			);
			return;
		}
		await backend.boardState.deleteBoard(app.data.id, boardId);
		await boards.refetch();
	};

	return (
		<main className="h-full flex flex-col max-h-full overflow-auto md:overflow-visible min-h-0">
			<div className="container mx-auto px-6 pb-4 flex flex-col h-full gap-4">
				<PermissionGate
					appId={id}
					require={[RolePermissions.ReadBoards]}
					feature={t("flows", "Flows")}
					description={t(
						"yourRoleCannotListTheFlowsInThisProjectSoThisPageCannotTellYouWhetherAnyExist",
						"Your role cannot list the flows in this project, so this page cannot tell you whether any exist.",
					)}
				>
					<FlowsOverviewPage
						appId={id ?? ""}
						app={app.data}
						boards={boards}
						boardCreation={boardCreation}
						setBoardCreation={setBoardCreation}
						onCreateBoard={handleCreateBoard}
						onOpenBoard={handleOpenBoard}
						onDeleteBoard={handleDeleteBoard}
						boardHref={
							app.data
								? (boardId) => `/flow?id=${boardId}&app=${app.data.id}`
								: undefined
						}
						pageHref={
							app.data
								? (pageId, boardId) =>
										`/page-builder?id=${pageId}&app=${app.data.id}&board=${boardId}`
								: undefined
						}
						eventsHref={id ? `/library/config/events?id=${id}` : undefined}
					/>
				</PermissionGate>
			</div>
		</main>
	);
}
