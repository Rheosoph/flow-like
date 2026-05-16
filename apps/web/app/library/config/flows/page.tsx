"use client";
import { createId } from "@paralleldrive/cuid2";
import {
	type FlowLibraryBoardCreationState,
	FlowLibraryBoardsSection,
	FlowLibraryHeader,
	IExecutionStage,
	ILogLevel,
	useBackend,
	useFlowBoardParentState,
	useInvoke,
} from "@tm9657/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";

export default function Page() {
	const backend = useBackend();
	const parentRegister = useFlowBoardParentState();
	const searchParams = useSearchParams();
	const id = searchParams.get("id");
	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[id ?? ""],
		typeof id === "string",
	);
	const boards = useInvoke(
		backend.boardState.getBoards,
		backend.boardState,
		[id ?? ""],
		typeof id === "string",
	);

	useEffect(() => {
		if (!app.data) return;
		if (!boards.data) return;
		boards.data?.forEach((board) => {
			parentRegister?.addBoardParent(
				board.id,
				`/library/config/flows?id=${id}`,
			);
		});
	}, [boards.data, id]);

	const router = useRouter();
	const [boardCreation, setBoardCreation] =
		useState<FlowLibraryBoardCreationState>({
			open: false,
			name: "",
			description: "",
		});

	const handleCreateBoard = async () => {
		if (!id) return;
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
		// In web mode, navigate directly to the flow editor - board data is loaded from backend
		router.push(`/flow?id=${boardId}&app=${app.data.id}`);
	};

	const handleDeleteBoard = async (boardId: string) => {
		if (!app.data) return;
		await backend.boardState.deleteBoard(app.data.id, boardId);
		await boards.refetch();
	};

	return (
		<main className="h-full flex flex-col max-h-full overflow-auto md:overflow-visible min-h-0">
			<div className="container mx-auto px-6 pb-4 flex flex-col h-full gap-4">
				<FlowLibraryHeader
					boardCreation={boardCreation}
					setBoardCreation={setBoardCreation}
					onCreateBoard={handleCreateBoard}
				/>

				<FlowLibraryBoardsSection
					boards={boards}
					app={app.data}
					boardCreation={boardCreation}
					setBoardCreation={setBoardCreation}
					onOpenBoard={handleOpenBoard}
					onDeleteBoard={handleDeleteBoard}
					boardHref={
						app.data
							? (boardId) => `/flow?id=${boardId}&app=${app.data.id}`
							: undefined
					}
				/>
			</div>
		</main>
	);
}
