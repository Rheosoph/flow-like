"use client";
import { useDraggable } from "@dnd-kit/core";
import { createId } from "@paralleldrive/cuid2";
import {
	CirclePlusIcon,
	GripIcon,
	PencilIcon,
	SettingsIcon,
	SquareFunctionIcon,
	Trash2Icon,
} from "lucide-react";
import { type RefObject, useCallback, useMemo, useState } from "react";
import { Button } from "../../../components/ui/button";
import { Input } from "../../../components/ui/input";
import {
	type IGenericCommand,
	removeLayerCommand,
	upsertLayerCommand,
} from "../../../lib";
import {
	type IBoard,
	type ILayer,
	ILayerType,
} from "../../../lib/schema/flow/board";
import { LayerEditMenu } from "../layer-editing-menu";

export function useCreateFunction(
	executeCommand: (command: IGenericCommand, append: boolean) => Promise<any>,
) {
	return useCallback(async () => {
		const id = createId();
		const layer: ILayer = {
			id,
			name: "New Function",
			type: ILayerType.Function,
			coordinates: [0, 0, 0],
			nodes: {},
			pins: {},
			variables: {},
			comments: {},
			parent_id: null,
			color: null,
			comment: null,
			error: null,
		};
		const command = upsertLayerCommand({ layer, node_ids: [] });
		await executeCommand(command, false);
	}, [executeCommand]);
}

export function FunctionsList({
	board,
	executeCommand,
	pushLayer,
	boardRef,
}: Readonly<{
	board: IBoard;
	executeCommand: (command: IGenericCommand, append: boolean) => Promise<any>;
	pushLayer: (layer: ILayer) => Promise<void>;
	boardRef?: RefObject<IBoard | undefined>;
}>) {
	const [editingLayer, setEditingLayer] = useState<ILayer | null>(null);

	const functions = useMemo(
		() =>
			Object.values(board.layers).filter(
				(l) => l.type === ILayerType.Function,
			),
		[board.layers],
	);

	const removeFunction = useCallback(
		async (layer: ILayer) => {
			const command = removeLayerCommand({
				layer,
				preserve_nodes: false,
				child_layers: [],
				layer_nodes: [],
				layers: [],
				nodes: [],
			});
			await executeCommand(command, false);
		},
		[executeCommand],
	);

	const renameFunction = useCallback(
		async (layer: ILayer, name: string) => {
			const command = upsertLayerCommand({
				layer: { ...layer, name },
				node_ids: [],
			});
			await executeCommand(command, false);
		},
		[executeCommand],
	);

	if (functions.length === 0) {
		return (
			<p className="text-xs text-muted-foreground py-1">
				No functions yet.
			</p>
		);
	}

	return (
		<>
			<div className="flex flex-col gap-1">
				{functions.map((fn) => (
					<FunctionItem
						key={fn.id}
						layer={fn}
						onNavigate={() => pushLayer(fn)}
						onRename={(name) => renameFunction(fn, name)}
						onEdit={() => setEditingLayer(fn)}
						onDelete={() => removeFunction(fn)}
					/>
				))}
			</div>

			{editingLayer && (
				<LayerEditMenu
					open={!!editingLayer}
					layer={editingLayer}
					onOpenChange={(open) => {
						if (!open) setEditingLayer(null);
					}}
					boardRef={boardRef}
					onApply={async (updated) => {
						const newLayer = { ...editingLayer, pins: updated.pins };
						const command = upsertLayerCommand({
							layer: newLayer,
							node_ids: [],
						});
						await executeCommand(command, false);
						setEditingLayer(null);
					}}
				/>
			)}
		</>
	);
}

function FunctionItem({
	layer,
	onNavigate,
	onRename,
	onEdit,
	onDelete,
}: Readonly<{
	layer: ILayer;
	onNavigate: () => void;
	onRename: (name: string) => void;
	onEdit: () => void;
	onDelete: () => void;
}>) {
	const [isRenaming, setIsRenaming] = useState(false);
	const [nameValue, setNameValue] = useState(layer.name);

	const { attributes, listeners, setNodeRef, transform } = useDraggable({
		id: `function-${layer.id}`,
		data: { type: "function-layer", layerId: layer.id },
	});

	const style = transform
		? { transform: `translate3d(${transform.x}px, ${transform.y}px, 0)` }
		: undefined;

	const inputCount = Object.values(layer.pins).filter(
		(p) => p.pin_type === "Input",
	).length;
	const outputCount = Object.values(layer.pins).filter(
		(p) => p.pin_type === "Output",
	).length;

	const commitRename = () => {
		const trimmed = nameValue.trim();
		if (trimmed && trimmed !== layer.name) {
			onRename(trimmed);
		} else {
			setNameValue(layer.name);
		}
		setIsRenaming(false);
	};

	return (
		<div
			ref={setNodeRef}
			style={style}
			className="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-muted/50 group"
		>
			<div
				className="cursor-grab active:cursor-grabbing text-muted-foreground"
				{...attributes}
				{...listeners}
			>
				<GripIcon className="w-3 h-3" />
			</div>

			<SquareFunctionIcon className="w-4 h-4 shrink-0 text-violet-500" />

			{isRenaming ? (
				<Input
					autoFocus
					className="h-6 text-sm px-1 flex-1"
					value={nameValue}
					onChange={(e) => setNameValue(e.target.value)}
					onBlur={commitRename}
					onKeyDown={(e) => {
						if (e.key === "Enter") commitRename();
						if (e.key === "Escape") {
							setNameValue(layer.name);
							setIsRenaming(false);
						}
					}}
				/>
			) : (
				<button
					type="button"
					className="flex-1 text-left min-w-0 cursor-pointer"
					onClick={onNavigate}
				>
					<span className="text-sm font-medium truncate block">
						{layer.name}
					</span>
					{(inputCount > 0 || outputCount > 0) && (
						<span className="text-xs text-muted-foreground">
							{inputCount} in / {outputCount} out
						</span>
					)}
				</button>
			)}

			<div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
				<Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => { setNameValue(layer.name); setIsRenaming(true); }}>
					<PencilIcon className="w-3 h-3" />
				</Button>
				<Button variant="ghost" size="icon" className="h-6 w-6" onClick={onEdit}>
					<SettingsIcon className="w-3 h-3" />
				</Button>
				<Button variant="ghost" size="icon" className="h-6 w-6 text-destructive" onClick={onDelete}>
					<Trash2Icon className="w-3 h-3" />
				</Button>
			</div>
		</div>
	);
}
