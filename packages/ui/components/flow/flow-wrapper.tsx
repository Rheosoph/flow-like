import { DndContext, MouseSensor, useSensor, useSensors } from "@dnd-kit/core";
import type { ReactNode } from "react";
import { useCallback, useMemo, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { snapshotFromBoard } from "../../lib/learn/board-bridge";
import type { IVariable } from "../../lib/schema/flow/variable";
import { useBackend } from "../../state/backend-state";
import { BoardBridgeResponder } from "../learn/board-bridge-responder";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../ui/dialog";
import { FlowBoard } from "./flow-board";

export function FlowWrapper({
	boardId,
	appId,
	nodeId,
	version,
	extraDockItems,
	renderOverlay,
	sub,
}: Readonly<{
	boardId: string;
	appId: string;
	nodeId?: string;
	version?: [number, number, number];
	extraDockItems?: Array<{
		title: string;
		icon: ReactNode;
		onClick: () => Promise<void> | void;
		separator?: string;
		highlight?: boolean;
		special?: boolean;
	}>;
	renderOverlay?: () => ReactNode;
	/** The authenticated user's sub (subject) from the auth token - used for realtime collaboration */
	sub?: string;
}>) {
	const mouseSensor = useSensor(MouseSensor, {
		activationConstraint: {
			distance: 10,
		},
	});

	const [detail, setDetail] = useState<
		| undefined
		| {
				variable: IVariable;
				screenPosition: { x: number; y: number };
		  }
	>();

	const sensors = useSensors(mouseSensor);

	const backend = useBackend();
	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, boardId, version],
		boardId !== "" && appId !== "",
	);
	const snapshotFn = useMemo(
		() => () => (board.data ? snapshotFromBoard(appId, board.data) : null),
		[board.data, appId],
	);

	const placeNode = useCallback(
		async (operation: "set" | "get") => {
			document.dispatchEvent(
				new CustomEvent("flow-drop", {
					detail: { ...detail, operation },
				}),
			);
			setDetail(undefined);
		},
		[detail, boardId],
	);

	return (
		<DndContext
			sensors={sensors}
			onDragEnd={(event) => {
				if (!event.over) return;
				const overId = String(event.over.id);
				const data = event.active.data.current;
				if (!data) return;

				// Function layer dropped on the canvas -> place CallFunction node directly
				if (data.type === "function-layer" && overId === "flow") {
					const mouseEvent = event.activatorEvent as MouseEvent;
					document.dispatchEvent(
						new CustomEvent("flow-drop", {
							detail: {
								type: "function-layer",
								layerId: data.layerId,
								screenPosition: {
									x: mouseEvent.screenX + event.delta.x,
									y: mouseEvent.screenY + event.delta.y,
								},
							},
						}),
					);
					return;
				}

				const variable = data as IVariable | undefined;
				if (!variable) return;

				// Dropped on the canvas -> ask user whether to Get/Set
				if (overId === "flow") {
					const mouseEvent: MouseEvent = event.activatorEvent as MouseEvent;
					setDetail({
						variable,
						screenPosition: {
							x: mouseEvent.screenX + event.delta.x,
							y: mouseEvent.screenY + event.delta.y,
						},
					});
					return;
				}

				// Dropped on a folder or root -> broadcast to VariablesMenu
				document.dispatchEvent(
					new CustomEvent("variables-folder-drop", {
						detail: {
							variable,
							targetPath: overId, // "__root" for top-level
						},
					}),
				);
			}}
		>
			<FlowBoard
				boardId={boardId}
				appId={appId}
				nodeId={nodeId}
				initialVersion={version}
				extraDockItems={extraDockItems}
				renderOverlay={renderOverlay}
				sub={sub}
			/>
			<BoardBridgeResponder
				snapshot={snapshotFn}
				announce={{ appId, boardId }}
			/>
			<Dialog
				open={detail !== undefined}
				onOpenChange={(open) => {
					if (!open) setDetail(undefined);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Reference: {detail?.variable.name}</DialogTitle>
					</DialogHeader>
					<div className="w-full flex items-center justify-start gap-2 max-w-full">
						<Button
							className="flex-grow"
							variant={"outline"}
							onClick={() => placeNode("get")}
						>
							Get
						</Button>
						<Button
							className="flex-grow"
							variant={"outline"}
							onClick={() => placeNode("set")}
						>
							Set
						</Button>
					</div>
				</DialogContent>
			</Dialog>
		</DndContext>
	);
}
