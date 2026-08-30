"use client";
import type { RefObject } from "react";
import { useCallback, useMemo } from "react";
import { useInvalidateInvoke } from "../../../hooks";
import {
	type IGenericCommand,
	removeLayerCommand,
	removeVariableCommand,
	upsertLayerCommand,
	upsertVariableCommand,
} from "../../../lib";
import type { IBoard, ILayer, IVariable } from "../../../lib/schema/flow/board";
import { ILayerType } from "../../../lib/schema/flow/board";
import type { INode } from "../../../lib/schema/flow/node";
import { useBackendStore } from "../../../state/backend-state";
import { useUndoRedo } from "../flow-history";
import { FunctionOverlay } from "../functions/function-overlay";
import {
	buildFolderTree,
	buildUsageIndex,
	folderPaths,
	functionLayers,
	refFromPin,
	resolveVariableScope,
} from "../token-board/model";
import { VariableOverlay } from "../variables/variable-overlay";

/** Node names whose hover toolbar offers the same editor the sidebar tabs use. */
export const EDITABLE_REFERENCE_NODES = new Set([
	"variable_get",
	"variable_set",
	"control_call_function",
]);

/**
 * What the hover toolbar's Edit button opens for a reference node.
 *
 * The local variant carries its function layer id: a local variable saved
 * without one would silently move to board scope.
 */
export type INodeEditTarget =
	| { kind: "variable"; variable: IVariable; scope: "board" }
	| { kind: "variable"; variable: IVariable; scope: "local"; layerId: string }
	| { kind: "function"; layer: ILayer };

/**
 * The function layer that owns the local variable scope at `layerId`.
 *
 * Local variables live on the function layer, but the canvas can be standing in
 * a plain group nested inside it — walking up is the only way to find the scope
 * a `variable_get` in that group actually reads from.
 */
function enclosingFunctionLayer(
	board: IBoard,
	layerId: string | undefined,
): ILayer | undefined {
	const seen = new Set<string>();
	let current = layerId ? board.layers?.[layerId] : undefined;
	while (current && !seen.has(current.id)) {
		if (current.type === ILayerType.Function) return current;
		seen.add(current.id);
		current = current.parent_id ? board.layers?.[current.parent_id] : undefined;
	}
	return undefined;
}

/**
 * Resolves what a reference node points at, or `undefined` when the reference is
 * dangling — the caller keeps the editor closed rather than opening it empty.
 */
export function resolveNodeEditTarget(
	node: INode,
	board: IBoard | undefined,
	currentLayerId: string | undefined,
): INodeEditTarget | undefined {
	if (!board) return undefined;

	if (node.name === "variable_get" || node.name === "variable_set") {
		const id = refFromPin(node, "var_ref");
		if (!id) return undefined;
		const functionLayer = enclosingFunctionLayer(board, currentLayerId);
		const scope = resolveVariableScope(
			id,
			functionLayer?.variables,
			board.variables,
		);
		if (!scope) return undefined;
		if (scope === "local") {
			const variable = functionLayer?.variables?.[id];
			if (!variable || !functionLayer) return undefined;
			return { kind: "variable", variable, scope, layerId: functionLayer.id };
		}
		const variable = board.variables?.[id];
		if (!variable) return undefined;
		return { kind: "variable", variable, scope };
	}

	if (node.name === "control_call_function") {
		const id = refFromPin(node, "function_layer_id");
		const layer = id ? board.layers?.[id] : undefined;
		if (layer?.type !== ILayerType.Function) return undefined;
		return { kind: "function", layer };
	}

	return undefined;
}

export interface IFlowNodeEditMenuProps {
	target: INodeEditTarget;
	appId: string;
	boardId: string;
	boardRef?: RefObject<IBoard | undefined>;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Navigates the canvas into a function layer; absent on surfaces that can't. */
	onOpenLayer?: (layer: ILayer) => void;
}

/**
 * The sidebar's variable and function editors, reachable from the node that
 * references them — same overlays, same commands, so the two entry points can
 * never drift apart.
 */
export function FlowNodeEditMenu({
	target,
	appId,
	boardId,
	boardRef,
	open,
	onOpenChange,
	onOpenLayer,
}: Readonly<IFlowNodeEditMenuProps>) {
	const invalidate = useInvalidateInvoke();
	const { pushCommand } = useUndoRedo(appId, boardId);
	const board = boardRef?.current;

	const execute = useCallback(
		async (command: IGenericCommand) => {
			const backend = useBackendStore.getState().backend;
			if (!backend) return;
			const result = await backend.boardState.executeCommand(
				appId,
				boardId,
				command,
			);
			await pushCommand(result, false);
			await invalidate(backend.boardState.getBoard, [appId, boardId]);
		},
		[appId, boardId, invalidate, pushCommand],
	);

	const usage = useMemo(() => buildUsageIndex(board), [board]);

	const folders = useMemo(() => {
		const source =
			target.kind === "variable"
				? Object.values(board?.variables ?? {}).map((variable) => ({
						id: variable.id,
						name: variable.name,
						category: variable.category,
						kind: "variable" as const,
						variable,
						uses: 0,
						scope: "board" as const,
					}))
				: board
					? functionLayers(board).map((layer) => ({
							id: layer.id,
							name: layer.name,
							category: layer.category,
							kind: "function" as const,
							layer,
							uses: 0,
							scope: "board" as const,
						}))
					: [];
		return folderPaths(buildFolderTree(source));
	}, [board, target.kind]);

	if (target.kind === "variable") {
		const { variable } = target;
		// Scope comes from the resolved target, not the overlay callback, so the
		// layer id and the scope it belongs to can never disagree.
		const scoped = target.scope === "local" ? { layer_id: target.layerId } : {};

		return (
			<VariableOverlay
				key={variable.id}
				open={open}
				onOpenChange={onOpenChange}
				variable={variable}
				scope={target.scope}
				uses={usage.variables[variable.id] ?? 0}
				folders={folders}
				refs={board?.refs}
				onApply={async (updated) => {
					if (!updated.editable) return;
					await execute(
						upsertVariableCommand({ variable: updated, ...scoped }),
					);
				}}
				onDelete={(removed) => {
					if (!removed.editable) return;
					onOpenChange(false);
					void execute(removeVariableCommand({ variable: removed, ...scoped }));
				}}
			/>
		);
	}

	const { layer } = target;

	return (
		<FunctionOverlay
			key={layer.id}
			open={open}
			onOpenChange={onOpenChange}
			layer={layer}
			calls={usage.functions[layer.id] ?? 0}
			folders={folders}
			boardRef={boardRef}
			onApply={async (updated) => {
				await execute(upsertLayerCommand({ layer: updated, node_ids: [] }));
			}}
			onDelete={() => {
				onOpenChange(false);
				void execute(
					removeLayerCommand({
						layer,
						preserve_nodes: false,
						child_layers: [],
						layer_nodes: [],
						layers: [],
						nodes: [],
					}),
				);
			}}
			onOpenLayer={() => {
				onOpenChange(false);
				onOpenLayer?.(layer);
			}}
		/>
	);
}
