"use client";
import { useQueryClient } from "@tanstack/react-query";
import {
	addNodeCommand,
	type INode,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import { useSearchParams } from "next/navigation";
import { useEffect, useRef } from "react";
import { toast } from "sonner";

interface FlowDeepLinkHandlerProps {
	readonly appId: string;
	readonly boardId: string;
}

/**
 * Watches the /flow URL for `?addNode=...&coordsX=...&coordsY=...` parameters
 * and applies them to the loaded board exactly once. Used by the University
 * lesson runtime's "Add this node" button.
 */
export function FlowDeepLinkHandler({
	appId,
	boardId,
}: FlowDeepLinkHandlerProps) {
	const params = useSearchParams();
	const backend = useBackend();
	const queryClient = useQueryClient();
	const consumed = useRef<string | null>(null);

	const board = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId, boardId, undefined],
		boardId !== "" && appId !== "",
	);
	const catalog = useInvoke(
		backend.boardState.getCatalog,
		backend.boardState,
		[appId],
		appId !== "",
	);

	useEffect(() => {
		if (!board.data || !catalog.data) return;
		const addNodeName = params.get("addNode");
		if (!addNodeName) return;
		const key = `${appId}|${boardId}|${addNodeName}|${params.get("coordsX") ?? ""}|${params.get("coordsY") ?? ""}`;
		if (consumed.current === key) return;
		consumed.current = key;

		const prototype = (catalog.data as INode[]).find(
			(n) => n.name === addNodeName,
		);
		if (!prototype) {
			toast.error(`Node "${addNodeName}" not found in this app's catalog`);
			return;
		}

		const x = Number(params.get("coordsX") ?? "0");
		const y = Number(params.get("coordsY") ?? "0");
		const layerId = params.get("layer") ?? null;

		const cloned: INode = {
			...prototype,
			id: prototype.id,
			coordinates: [x, y, 0],
			pins: { ...prototype.pins },
		};

		const result = addNodeCommand({ node: cloned, current_layer: layerId });

		(async () => {
			try {
				await backend.boardState.executeCommand(appId, boardId, result.command);
				await queryClient.invalidateQueries({
					queryKey: ["getBoard", appId, boardId],
				});
				toast.success(
					`Added "${prototype.friendly_name ?? prototype.name}" to the board`,
				);
			} catch (err) {
				console.error("addNode deep link failed", err);
				toast.error("Could not add the node — see console for details.");
			}
		})();
	}, [
		appId,
		boardId,
		params,
		board.data,
		catalog.data,
		backend.boardState,
		queryClient,
	]);

	return null;
}
