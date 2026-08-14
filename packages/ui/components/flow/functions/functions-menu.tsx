"use client";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { useDraggable } from "@dnd-kit/core";
import { createId } from "@paralleldrive/cuid2";
import {
	FolderInputIcon,
	GripIcon,
	PencilIcon,
	SettingsIcon,
	SquareFunctionIcon,
	Trash2Icon,
} from "lucide-react";
import {
	type RefObject,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import { Button } from "../../../components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../../components/ui/dialog";
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
import {
	CategoryTree,
	FOLDER_DROP_EVENT,
	type IFolderDropDetail,
	buildCategoryTree,
	collectFolderPaths,
	normalizeCategory,
} from "../category-tree";
import { LayerEditMenu } from "../layer-editing-menu";

const FUNCTION_FOLDER_KIND = "functions";

export function useCreateFunction(
	executeCommand: (command: IGenericCommand, append: boolean) => Promise<any>,
) {
	return useCallback(async () => {
		const id = createId();
		const layer: ILayer = {
			id,
			name: i18next.t('newFunction', 'New Function'),
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
	const { t } = useTranslation("flow");
	const [editingLayer, setEditingLayer] = useState<ILayer | null>(null);
	const [movingLayer, setMovingLayer] = useState<ILayer | null>(null);

	const functions = useMemo(
		() =>
			Object.values(board.layers).filter((l) => l.type === ILayerType.Function),
		[board.layers],
	);

	const tree = useMemo(() => buildCategoryTree(functions), [functions]);
	const folders = useMemo(() => collectFolderPaths(tree), [tree]);

	const upsertFunction = useCallback(
		async (layer: ILayer) => {
			const command = upsertLayerCommand({ layer, node_ids: [] });
			await executeCommand(command, false);
		},
		[executeCommand],
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
			await upsertFunction({ ...layer, name });
		},
		[upsertFunction],
	);

	const moveFunction = useCallback(
		async (layer: ILayer, category?: string) => {
			if (normalizeCategory(layer.category) === category) return;
			await upsertFunction({ ...layer, category: category ?? null });
		},
		[upsertFunction],
	);

	// Folder drops are dispatched by FlowWrapper, which owns the DndContext.
	useEffect(() => {
		const handler = (event: Event) => {
			const detail = (
				event as CustomEvent<IFolderDropDetail<{ layerId?: string }>>
			).detail;
			if (!detail || detail.consumed) return;
			if (detail.kind !== FUNCTION_FOLDER_KIND) return;

			const layerId = detail.item?.layerId;
			const layer = layerId ? board.layers[layerId] : undefined;
			if (layer?.type !== ILayerType.Function) return;

			detail.consumed = true;
			void moveFunction(layer, normalizeCategory(detail.path));
		};
		document.addEventListener(FOLDER_DROP_EVENT, handler);
		return () => document.removeEventListener(FOLDER_DROP_EVENT, handler);
	}, [board.layers, moveFunction]);

	if (functions.length === 0) {
		return (
			<p className="text-xs text-muted-foreground py-1">{t('noFunctionsYet', 'No functions yet.')}</p>
		);
	}

	return (
		<>
			<CategoryTree
				root={tree}
				kind={FUNCTION_FOLDER_KIND}
				renderItem={(fn) => (
					<FunctionItem
						layer={fn}
						onNavigate={() => pushLayer(fn)}
						onRename={(name) => renameFunction(fn, name)}
						onEdit={() => setEditingLayer(fn)}
						onMove={() => setMovingLayer(fn)}
						onDelete={() => removeFunction(fn)}
					/>
				)}
			/>

			{movingLayer && (
				<MoveToFolderDialog
					open={!!movingLayer}
					onOpenChange={(open) => {
						if (!open) setMovingLayer(null);
					}}
					layer={movingLayer}
					folders={folders}
					onMove={async (category) => {
						await moveFunction(movingLayer, category);
						setMovingLayer(null);
					}}
				/>
			)}

			{editingLayer && (
				<LayerEditMenu
					open={!!editingLayer}
					layer={editingLayer}
					onOpenChange={(open) => {
						if (!open) setEditingLayer(null);
					}}
					boardRef={boardRef}
					onApply={async (updated) => {
						const newLayer = {
							...editingLayer,
							pins: updated.pins,
							cache: (updated as ILayer).cache,
						};
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
	onMove,
	onDelete,
}: Readonly<{
	layer: ILayer;
	onNavigate: () => void;
	onRename: (name: string) => void;
	onEdit: () => void;
	onMove: () => void;
	onDelete: () => void;
}>) {
	const { t } = useTranslation("flow");
	const [isRenaming, setIsRenaming] = useState(false);
	const [nameValue, setNameValue] = useState(layer.name);

	const { attributes, listeners, setNodeRef, transform } = useDraggable({
		id: `function-${layer.id}`,
		data: { type: "function-layer", layerId: layer.id },
	});

	const style = transform
		? { transform: t('translate3dxpxYpx0', 'translate3d({{x}}px, {{y}}px, 0)', { x: transform.x, y: transform.y }) }
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
						<span className="text-xs text-muted-foreground">{t('inputcountInOutputcountOut', '{{inputCount}} in / {{outputCount}} out', { inputCount, outputCount })}</span>
					)}
				</button>
			)}

			<div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
				<Button
					variant="ghost"
					size="icon"
					className="h-6 w-6"
					onClick={() => {
						setNameValue(layer.name);
						setIsRenaming(true);
					}}
				>
					<PencilIcon className="w-3 h-3" />
				</Button>
				<Button
					variant="ghost"
					size="icon"
					className="h-6 w-6"
					title={t('moveToFolder', 'Move to folder')}
					onClick={onMove}
				>
					<FolderInputIcon className="w-3 h-3" />
				</Button>
				<Button
					variant="ghost"
					size="icon"
					className="h-6 w-6"
					onClick={onEdit}
				>
					<SettingsIcon className="w-3 h-3" />
				</Button>
				<Button
					variant="ghost"
					size="icon"
					className="h-6 w-6 text-destructive"
					onClick={onDelete}
				>
					<Trash2Icon className="w-3 h-3" />
				</Button>
			</div>
		</div>
	);
}

function MoveToFolderDialog({
	open,
	onOpenChange,
	layer,
	folders,
	onMove,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	layer: ILayer;
	folders: string[];
	onMove: (category?: string) => Promise<void>;
}>) {
	const { t } = useTranslation("flow");
	const [draft, setDraft] = useState(layer.category ?? "");

	const commit = useCallback(() => {
		void onMove(normalizeCategory(draft));
	}, [draft, onMove]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<FolderInputIcon className="h-4 w-4 text-primary" />{`Move “${layer.name}”`}</DialogTitle>
					<DialogDescription>
						{t('useToCreateNestedFoldersLeaveEmptyForToplevel', 'Use “/” to create nested folders. Leave empty for top-level.')}
					</DialogDescription>
				</DialogHeader>

				<Input
					autoFocus
					value={draft}
					onChange={(e) => setDraft(e.target.value)}
					onKeyDown={(e) => {
						if (e.key === "Enter") commit();
					}}
					placeholder={t('egUtilsmath', 'e.g. Utils/Math')}
				/>

				{folders.length > 0 && (
					<div className="flex flex-wrap gap-1">
						<Button
							variant="outline"
							size="sm"
							className="h-6 text-xs"
							onClick={() => setDraft("")}
						>
							{t('topLevel', 'Top level')}
						</Button>
						{folders.map((folder) => (
							<Button
								key={folder}
								variant="outline"
								size="sm"
								className="h-6 text-xs"
								onClick={() => setDraft(folder)}
							>
								{folder}
							</Button>
						))}
					</div>
				)}

				<DialogFooter className="gap-2">
					<Button variant="secondary" onClick={() => onOpenChange(false)}>
						{t('cancel', 'Cancel')}
					</Button>
					<Button onClick={commit}>{t('move', 'Move')}</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
