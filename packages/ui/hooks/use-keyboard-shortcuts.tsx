import type { UseQueryResult } from "@tanstack/react-query";
import { useQueryClient } from "@tanstack/react-query";
import { Redo2Icon, Undo2Icon, XIcon } from "lucide-react";
import { type RefObject, useCallback, useEffect } from "react";
import { toastError, toastSuccess } from "../lib/messages";
import type { IGenericCommand } from "../lib/schema";
import type { IBoard } from "../lib/schema/flow/board";
import type { INode } from "../lib/schema/flow/node";
import { useBackend } from "../state/backend-state";

interface UseKeyboardShortcutsProps {
	board: UseQueryResult<IBoard>;
	catalog: UseQueryResult<INode[]>;
	version: [number, number, number] | undefined;
	appId: string;
	boardId: string;
	mousePositionRef: RefObject<{ x: number; y: number }>;
	onDeleteSelection: () => Promise<void>;
	placeNode: (
		node: INode,
		position?: { x: number; y: number },
	) => Promise<void>;
	undo: () => Promise<IGenericCommand[] | null>;
	redo: () => Promise<IGenericCommand[] | null>;
	rollbackUndo: (commands: IGenericCommand[]) => Promise<void>;
	rollbackRedo: (commands: IGenericCommand[]) => Promise<void>;
}

export function useKeyboardShortcuts({
	board,
	catalog,
	version,
	appId,
	boardId,
	mousePositionRef,
	onDeleteSelection,
	placeNode,
	undo,
	redo,
	rollbackUndo,
	rollbackRedo,
}: UseKeyboardShortcutsProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();

	// Helper to invalidate and refetch board data
	const invalidateBoard = useCallback(async () => {
		const queryKey = ["getBoard", appId, boardId, version].filter(
			(arg) => typeof arg !== "undefined",
		);
		await queryClient.invalidateQueries({ queryKey });
		await board.refetch();
	}, [queryClient, appId, boardId, version, board]);

	const placeNodeShortcut = useCallback(
		async (node: INode) => {
			const mp = mousePositionRef.current;
			await placeNode(node, {
				x: mp.x,
				y: mp.y,
			});
		},
		[placeNode],
	);

	const shortcutHandler = useCallback(
		async (event: KeyboardEvent) => {
			if (event.repeat) return;

			const target = event.target as HTMLElement;
			if (
				target.tagName === "INPUT" ||
				target.tagName === "TEXTAREA" ||
				target.isContentEditable
			) {
				return;
			}

			if (
				(event.key === "Backspace" || event.key === "Delete") &&
				!event.metaKey &&
				!event.ctrlKey &&
				!event.altKey
			) {
				event.preventDefault();
				event.stopPropagation();
				if (typeof version !== "undefined") {
					toastError("Cannot change old version", <XIcon />);
					return;
				}
				await onDeleteSelection();
				return;
			}

			// Undo
			if (
				(event.metaKey || event.ctrlKey) &&
				event.key === "z" &&
				!event.shiftKey
			) {
				event.preventDefault();
				event.stopPropagation();
				if (typeof version !== "undefined") {
					toastError("Cannot change old version", <XIcon />);
					return;
				}
				const stack = await undo();
				if (stack) {
					try {
						await backend.boardState.undoBoard(appId, boardId, stack);
						await invalidateBoard();
						toastSuccess("Undo", <Undo2Icon className="w-4 h-4" />);
					} catch (error) {
						console.error("Undo failed:", error);
						await rollbackUndo(stack);
						toastError("Undo failed", <XIcon />);
						await invalidateBoard();
					}
				}
				return;
			}

			// Redo (Ctrl+Y / Cmd+Y or Ctrl+Shift+Z / Cmd+Shift+Z)
			if (
				(event.metaKey || event.ctrlKey) &&
				(event.key === "y" ||
					(event.key.toLowerCase() === "z" && event.shiftKey))
			) {
				event.preventDefault();
				event.stopPropagation();
				if (typeof version !== "undefined") {
					toastError("Cannot change old version", <XIcon />);
					return;
				}
				const stack = await redo();
				if (stack) {
					try {
						await backend.boardState.redoBoard(appId, boardId, stack);
						await invalidateBoard();
						toastSuccess("Redo", <Redo2Icon className="w-4 h-4" />);
					} catch (error) {
						console.error("Redo failed:", error);
						await rollbackRedo(stack);
						toastError("Redo failed", <XIcon />);
						await invalidateBoard();
					}
				}
				return;
			}

			// Place Branch
			if (
				(event.metaKey || event.ctrlKey) &&
				event.key === "b" &&
				!event.shiftKey
			) {
				event.preventDefault();
				event.stopPropagation();
				if (typeof version !== "undefined") {
					toastError("Cannot change old version", <XIcon />);
					return;
				}
				const node = catalog.data?.find(
					(node) => node.name === "control_branch",
				);
				if (!node) return;
				await placeNodeShortcut(node);
				await invalidateBoard();
				return;
			}

			// Place For Each
			if (
				(event.metaKey || event.ctrlKey) &&
				event.key === "f" &&
				!event.shiftKey
			) {
				event.preventDefault();
				event.stopPropagation();
				if (typeof version !== "undefined") {
					toastError("Cannot change old version", <XIcon />);
					return;
				}
				const node = catalog.data?.find(
					(node) => node.name === "control_for_each",
				);
				if (!node) return;
				await placeNodeShortcut(node);
				await invalidateBoard();
				return;
			}

			// Place Log Info
			if (
				(event.metaKey || event.ctrlKey) &&
				event.key === "p" &&
				!event.shiftKey
			) {
				event.preventDefault();
				event.stopPropagation();
				if (typeof version !== "undefined") {
					toastError("Cannot change old version", <XIcon />);
					return;
				}
				const node = catalog.data?.find((node) => node.name === "log_info");
				if (!node) return;
				await placeNodeShortcut(node);
				await invalidateBoard();
				return;
			}

			// Place Reroute
			if (
				(event.metaKey || event.ctrlKey) &&
				event.key === "s" &&
				!event.shiftKey
			) {
				event.preventDefault();
				event.stopPropagation();
				if (typeof version !== "undefined") {
					toastError("Cannot change old version", <XIcon />);
					return;
				}
				const node = catalog.data?.find((node) => node.name === "reroute");
				if (!node) return;
				await placeNodeShortcut(node);
				await invalidateBoard();
			}
		},
		[
			boardId,
			board,
			backend,
			version,
			catalog,
			placeNodeShortcut,
			undo,
			redo,
			rollbackUndo,
			rollbackRedo,
			appId,
			invalidateBoard,
			onDeleteSelection,
		],
	);

	useEffect(() => {
		document.addEventListener("keydown", shortcutHandler);
		return () => {
			document.removeEventListener("keydown", shortcutHandler);
		};
	}, [shortcutHandler]);
}
