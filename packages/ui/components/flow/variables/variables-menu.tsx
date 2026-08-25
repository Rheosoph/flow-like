"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	CirclePlusIcon,
	ContrastIcon,
	CornerUpLeftIcon,
	EllipsisVerticalIcon,
	GripIcon,
	HelpCircleIcon,
	ListIcon,
	SearchIcon,
	SquareFunctionIcon,
	XIcon,
} from "lucide-react";
import type { ReactNode, RefObject } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../../components/ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuSub,
	ContextMenuSubContent,
	ContextMenuSubTrigger,
	ContextMenuTrigger,
} from "../../../components/ui/context-menu";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../../components/ui/dropdown-menu";
import { Input } from "../../../components/ui/input";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "../../../components/ui/popover";
import {
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../../../components/ui/tabs";
import {
	type IGenericCommand,
	moveToLayerCommand,
	removeLayerCommand,
	removeVariableCommand,
	upsertLayerCommand,
	upsertVariableCommand,
} from "../../../lib";
import {
	MAIN_FILE_LABEL,
	activeModuleId,
	boardModules,
} from "../../../lib/flow-modules";
import { owningModuleId } from "../../../lib/layer-to-function";
import type { IBoard, ILayer, IVariable } from "../../../lib/schema/flow/board";
import { ILayerType } from "../../../lib/schema/flow/board";
import { IVariableType } from "../../../lib/schema/flow/node";
import { IValueType } from "../../../lib/schema/flow/pin";
import { cn } from "../../../lib/utils";
import {
	FOLDER_DROP_EVENT,
	type IFolderDropDetail,
	normalizeCategory,
} from "../category-tree";
import { FunctionOverlay } from "../functions/function-overlay";
import {
	type IGroupMode,
	IS_PREDICATES,
	type ITokenItem,
	TOKEN_GLYPH,
	buildFolderTree,
	buildUsageIndex,
	folderPaths,
	functionLayers,
	groupItemsByModule,
	matchesFunction,
	matchesVariable,
	parseTokenQuery,
	resolveVariableScope,
} from "../token-board/model";
import { FunctionToken, VariableToken } from "../token-board/token";
import { type ITokenSection, TokenBoard } from "../token-board/token-board";
import { typeToColor } from "../utils";
import { NewVariableDialog } from "./new-variable-dialog";
import { VariableOverlay } from "./variable-overlay";

/** Either a variable (variable trees) or a function-layer handle (function tree). */
type IDropPayload = IVariable & { layerId?: string };

const VARIABLE_FOLDER_KIND = "variables";
const LOCAL_VARIABLE_FOLDER_KIND = "local-variables";
const FUNCTION_FOLDER_KIND = "functions";

const pinCount = (layer: ILayer, pinType: "Input" | "Output") =>
	Object.values(layer.pins ?? {}).filter((pin) => pin.pin_type === pinType)
		.length;

/**
 * Creates a function layer. The name is asked for up front — the old button
 * dropped a silent "New Function" layer on the canvas and left you to find it.
 *
 * `moduleId` files the function inside a module, which is what makes it local to
 * that file instead of a board global.
 */
export function useCreateFunction(
	executeCommand: (
		command: IGenericCommand,
		append: boolean,
	) => Promise<unknown>,
) {
	return useCallback(
		async (name: string, category?: string, moduleId?: string | null) => {
			const parentId = moduleId ?? null;
			const layer: ILayer = {
				id: createId(),
				name,
				type: ILayerType.Function,
				coordinates: [0, 0, 0],
				nodes: {},
				pins: {},
				variables: {},
				comments: {},
				// The backend takes the parent of a *new* layer from `current_layer`;
				// `parent_id` is what every local reader goes by. Both, or the function
				// lands somewhere else than the file it was created from.
				parent_id: parentId,
				color: null,
				comment: null,
				error: null,
				category: normalizeCategory(category) ?? null,
			};
			await executeCommand(
				upsertLayerCommand({
					layer,
					node_ids: [],
					current_layer: parentId,
				}),
				false,
			);
			return layer;
		},
		[executeCommand],
	);
}

export function VariablesMenu({
	board,
	executeCommand,
	currentLayerId,
	pushLayer,
	boardRef,
}: Readonly<{
	board: IBoard;
	executeCommand: (
		command: IGenericCommand,
		append: boolean,
	) => Promise<unknown>;
	currentLayerId?: string;
	pushLayer?: (layer: ILayer) => Promise<void>;
	boardRef?: RefObject<IBoard | undefined>;
}>) {
	const { t } = useTranslation("flow");
	const [tab, setTab] = useState<"variables" | "functions">("variables");
	const [rawQuery, setRawQuery] = useState("");
	const [group, setGroup] = useState<IGroupMode>("folder");
	const [tint, setTint] = useState(true);
	const [editingVariable, setEditingVariable] = useState<IVariable | null>(
		null,
	);
	const [editingFunction, setEditingFunction] = useState<ILayer | null>(null);
	const [showNewVariableDialog, setShowNewVariableDialog] = useState(false);
	const [newVariableScope, setNewVariableScope] = useState<"board" | "local">(
		"board",
	);
	const [draftFunction, setDraftFunction] = useState<string | null>(null);
	const searchRef = useRef<HTMLInputElement | null>(null);
	const menuPosition = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
	const createFunction = useCreateFunction(executeCommand);

	const query = useMemo(() => parseTokenQuery(rawQuery), [rawQuery]);
	const usage = useMemo(() => buildUsageIndex(board), [board]);

	const currentFunctionLayer = useMemo(() => {
		if (!currentLayerId) return null;
		const layer = board.layers[currentLayerId];
		if (!layer || layer.type !== ILayerType.Function) return null;
		return layer;
	}, [currentLayerId, board.layers]);

	const modules = useMemo(() => boardModules(board.layers), [board.layers]);
	// The file the canvas is standing in — what a new function is filed into.
	const currentModuleId = useMemo(
		() => activeModuleId(undefined, currentLayerId, board.layers),
		[currentLayerId, board.layers],
	);

	/* ── commands ──────────────────────────────────────────────────────── */

	const upsertVariable = useCallback(
		async (variable: IVariable) => {
			await executeCommand(upsertVariableCommand({ variable }), false);
		},
		[executeCommand],
	);

	const upsertLocalVariable = useCallback(
		async (variable: IVariable) => {
			if (!currentFunctionLayer) return;
			await executeCommand(
				upsertVariableCommand({ variable, layer_id: currentFunctionLayer.id }),
				false,
			);
		},
		[currentFunctionLayer, executeCommand],
	);

	const saveVariable = useCallback(
		async (variable: IVariable, scope: "board" | "local") => {
			if (!variable.editable) return;
			if (scope === "local") await upsertLocalVariable(variable);
			else await upsertVariable(variable);
		},
		[upsertLocalVariable, upsertVariable],
	);

	const deleteVariable = useCallback(
		async (variable: IVariable, scope: "board" | "local") => {
			if (!variable.editable) return;
			const payload =
				scope === "local" && currentFunctionLayer
					? { variable, layer_id: currentFunctionLayer.id }
					: { variable };
			await executeCommand(removeVariableCommand(payload), false);
		},
		[currentFunctionLayer, executeCommand],
	);

	const upsertFunction = useCallback(
		async (layer: ILayer) => {
			await executeCommand(upsertLayerCommand({ layer, node_ids: [] }), false);
		},
		[executeCommand],
	);

	/** Re-files a function into another module — or into `main.flow`, which is no module. */
	const moveFunctionToModule = useCallback(
		async (layerId: string, target: string | null) => {
			await executeCommand(
				moveToLayerCommand({ ids: [layerId], target }),
				false,
			);
		},
		[executeCommand],
	);

	const deleteFunction = useCallback(
		async (layer: ILayer) => {
			await executeCommand(
				removeLayerCommand({
					layer,
					preserve_nodes: false,
					child_layers: [],
					layer_nodes: [],
					layers: [],
					nodes: [],
				}),
				false,
			);
		},
		[executeCommand],
	);

	/* ── items ─────────────────────────────────────────────────────────── */

	const variableItems = useMemo<ITokenItem[]>(() => {
		const items: ITokenItem[] = [];
		const push = (variable: IVariable, scope: "board" | "local") => {
			const uses = usage.variables[variable.id] ?? 0;
			if (!matchesVariable(variable, query, uses, scope)) return;
			items.push({
				id: variable.id,
				name: variable.name,
				category: variable.category,
				kind: "variable",
				variable,
				uses,
				scope,
			});
		};
		if (currentFunctionLayer) {
			for (const variable of Object.values(
				currentFunctionLayer.variables ?? {},
			))
				push(variable, "local");
		}
		for (const variable of Object.values(board.variables ?? {}))
			push(variable, "board");
		return items;
	}, [board.variables, currentFunctionLayer, query, usage]);

	const functionItems = useMemo<ITokenItem[]>(() => {
		return functionLayers(board)
			.map((layer) => ({ layer, calls: usage.functions[layer.id] ?? 0 }))
			.filter(({ layer, calls }) => matchesFunction(layer, query, calls))
			.map(({ layer, calls }) => ({
				id: layer.id,
				name: layer.name,
				category: layer.category,
				kind: "function" as const,
				layer,
				uses: calls,
				scope: "board" as const,
			}));
	}, [board, query, usage]);

	/**
	 * Functions grouped by the file they live in: globals first, then one section per
	 * module that owns at least one. A board whose functions are all global has a single
	 * group, and a single group is no grouping — it renders as it always did.
	 */
	const functionSections = useMemo<ITokenSection[] | undefined>(() => {
		if (modules.length === 0) return undefined;
		const labelOf = new Map(
			modules.map((module) => [module.id, module.pathLabel]),
		);
		const groups = groupItemsByModule(
			functionItems,
			board.layers,
			modules.map((module) => module.id),
		);
		if (groups.length === 0) return undefined;
		if (groups.length === 1 && groups[0].moduleId === null) return undefined;
		return groups.map((group) => ({
			key: group.moduleId ?? "global",
			label:
				group.moduleId === null
					? t("global", "Global")
					: (labelOf.get(group.moduleId) ?? group.moduleId),
			items: group.items,
		}));
	}, [board.layers, functionItems, modules, t]);

	const localItems = useMemo(
		() =>
			tab === "variables" && currentFunctionLayer && group === "folder"
				? variableItems.filter((item) => item.scope === "local")
				: [],
		[tab, currentFunctionLayer, group, variableItems],
	);
	const items = useMemo(() => {
		if (tab !== "variables") return functionItems;
		if (localItems.length === 0) return variableItems;
		return variableItems.filter((item) => item.scope !== "local");
	}, [tab, functionItems, variableItems, localItems]);
	const totalVariables =
		Object.keys(board.variables ?? {}).length +
		Object.keys(currentFunctionLayer?.variables ?? {}).length;
	const totalFunctions = functionLayers(board).length;

	const variableFolders = useMemo(
		() =>
			folderPaths(
				buildFolderTree(
					Object.values(board.variables ?? {}).map((variable) => ({
						id: variable.id,
						name: variable.name,
						category: variable.category,
						kind: "variable" as const,
						variable,
						uses: 0,
						scope: "board" as const,
					})),
				),
			),
		[board.variables],
	);

	const functionFolders = useMemo(
		() =>
			folderPaths(
				buildFolderTree(
					functionLayers(board).map((layer) => ({
						id: layer.id,
						name: layer.name,
						category: layer.category,
						kind: "function" as const,
						layer,
						uses: 0,
						scope: "board" as const,
					})),
				),
			),
		[board],
	);

	const unusedCount = items.filter((item) => item.uses === 0).length;

	/* ── folder drops (FlowWrapper owns the DndContext) ────────────────── */

	useEffect(() => {
		const handler = (event: Event) => {
			const detail = (event as CustomEvent<IFolderDropDetail<IDropPayload>>)
				.detail;
			if (!detail || detail.consumed) return;

			if (detail.kind === FUNCTION_FOLDER_KIND) {
				const layerId = detail.item?.layerId;
				const layer = layerId ? board.layers[layerId] : undefined;
				if (layer?.type !== ILayerType.Function) return;
				const next = normalizeCategory(detail.path) ?? null;
				if (normalizeCategory(layer.category) === (next ?? undefined)) return;
				detail.consumed = true;
				void upsertFunction({ ...layer, category: next });
				return;
			}

			if (
				detail.kind !== VARIABLE_FOLDER_KIND &&
				detail.kind !== LOCAL_VARIABLE_FOLDER_KIND
			)
				return;

			const variable = detail.item as IVariable;
			if (!variable?.editable) return;

			// Both scopes render in one board while you are inside a function, so the
			// droppable's kind cannot say which one a variable belongs to — only the
			// scope that actually holds it can.
			const owner = resolveVariableScope(
				variable.id,
				currentFunctionLayer?.variables,
				board.variables,
			);
			if (!owner) return;
			const isLocal = owner === "local";

			const nextCategory = normalizeCategory(detail.path);
			if (normalizeCategory(variable.category) === nextCategory) return;

			detail.consumed = true;
			const moved = { ...variable, category: nextCategory };
			void (isLocal ? upsertLocalVariable(moved) : upsertVariable(moved));
		};
		document.addEventListener(FOLDER_DROP_EVENT, handler);
		return () => document.removeEventListener(FOLDER_DROP_EVENT, handler);
	}, [
		board.layers,
		board.variables,
		currentFunctionLayer,
		upsertFunction,
		upsertLocalVariable,
		upsertVariable,
	]);

	/* ── actions ───────────────────────────────────────────────────────── */

	const insertNode = useCallback(
		(item: ITokenItem, operation: "get" | "set") => {
			const detail =
				item.kind === "function"
					? {
							type: "function-layer",
							layerId: item.id,
							screenPosition: menuPosition.current,
						}
					: {
							variable: item.variable,
							operation,
							screenPosition: menuPosition.current,
						};
			document.dispatchEvent(new CustomEvent("flow-drop", { detail }));
		},
		[],
	);

	const moveItem = useCallback(
		async (item: ITokenItem, category: string | null) => {
			if (item.kind === "function" && item.layer) {
				await upsertFunction({ ...item.layer, category });
				return;
			}
			if (!item.variable?.editable) return;
			await saveVariable(
				{ ...item.variable, category: category ?? undefined },
				item.scope,
			);
		},
		[saveVariable, upsertFunction],
	);

	const duplicateVariable = useCallback(
		async (item: ITokenItem) => {
			if (!item.variable) return;
			await saveVariable(
				{
					...item.variable,
					id: createId(),
					name: `${item.variable.name}_COPY`,
				},
				item.scope,
			);
		},
		[saveVariable],
	);

	const openFunction = useCallback(
		async (layer: ILayer) => {
			if (pushLayer) await pushLayer(layer);
		},
		[pushLayer],
	);

	const commitDraftFunction = useCallback(async () => {
		const name = (draftFunction ?? "").trim();
		if (!name) {
			setDraftFunction(null);
			return;
		}
		const layer = await createFunction(name, undefined, currentModuleId);
		setDraftFunction(null);
		setEditingFunction(layer);
	}, [createFunction, currentModuleId, draftFunction]);

	/* ── rendering ─────────────────────────────────────────────────────── */

	const folders = tab === "variables" ? variableFolders : functionFolders;

	const renderMenu = (item: ITokenItem) => {
		const itemModuleId =
			item.kind === "function" ? owningModuleId(board.layers, item.id) : null;

		return (
			<ContextMenuContent className="w-56">
				{item.kind === "variable" ? (
					<>
						<ContextMenuItem onClick={() => insertNode(item, "get")}>
							{t("insertGetNode", "Insert Get node")}
						</ContextMenuItem>
						<ContextMenuItem onClick={() => insertNode(item, "set")}>
							{t("insertSetNode", "Insert Set node")}
						</ContextMenuItem>
					</>
				) : (
					<>
						<ContextMenuItem onClick={() => insertNode(item, "get")}>
							{t("insertCallNode", "Insert Call node")}
						</ContextMenuItem>
						<ContextMenuItem
							onClick={() => item.layer && void openFunction(item.layer)}
						>
							{t("openLayer", "Open layer")}
						</ContextMenuItem>
					</>
				)}
				<ContextMenuSeparator />
				<ContextMenuItem
					onClick={() =>
						item.kind === "function"
							? setEditingFunction(item.layer ?? null)
							: setEditingVariable(item.variable ?? null)
					}
				>
					{t("editEllipsis", "Edit…")}
				</ContextMenuItem>
				{item.kind === "variable" && item.variable?.editable && (
					<ContextMenuItem
						onClick={() =>
							item.variable &&
							void saveVariable(
								{ ...item.variable, exposed: !item.variable.exposed },
								item.scope,
							)
						}
					>
						{item.variable.exposed
							? t("stopExposing", "Stop exposing")
							: t("exposeInAppConfig", "Expose in app config")}
					</ContextMenuItem>
				)}
				<ContextMenuSub>
					<ContextMenuSubTrigger>
						{t("moveToFolder", "Move to folder")}
					</ContextMenuSubTrigger>
					<ContextMenuSubContent className="max-h-64 overflow-y-auto">
						<ContextMenuItem onClick={() => void moveItem(item, null)}>
							{t("topLevel", "Top level")}
						</ContextMenuItem>
						{folders.map((path) => (
							<ContextMenuItem
								key={path}
								onClick={() => void moveItem(item, path)}
							>
								{path}
							</ContextMenuItem>
						))}
					</ContextMenuSubContent>
				</ContextMenuSub>
				{item.kind === "function" && modules.length > 0 && (
					<ContextMenuSub>
						<ContextMenuSubTrigger>
							{t("moveToModule", "Move to module")}
						</ContextMenuSubTrigger>
						<ContextMenuSubContent className="max-h-64 overflow-y-auto">
							<ContextMenuItem
								disabled={itemModuleId === null}
								onClick={() => void moveFunctionToModule(item.id, null)}
							>
								{MAIN_FILE_LABEL}
							</ContextMenuItem>
							{modules.map((module) => (
								<ContextMenuItem
									key={module.id}
									disabled={itemModuleId === module.id}
									onClick={() => void moveFunctionToModule(item.id, module.id)}
								>
									{module.pathLabel}
								</ContextMenuItem>
							))}
						</ContextMenuSubContent>
					</ContextMenuSub>
				)}
				{item.kind === "variable" && (
					<ContextMenuItem onClick={() => void duplicateVariable(item)}>
						{t("duplicate", "Duplicate")}
					</ContextMenuItem>
				)}
				<ContextMenuItem
					onClick={() => void navigator.clipboard?.writeText(item.name)}
				>
					{t("copyName", "Copy name")}
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem
					variant="destructive"
					disabled={item.kind === "variable" && !item.variable?.editable}
					onClick={() =>
						item.kind === "function"
							? item.layer && void deleteFunction(item.layer)
							: item.variable && void deleteVariable(item.variable, item.scope)
					}
				>
					{t("delete", "Delete")}
				</ContextMenuItem>
			</ContextMenuContent>
		);
	};

	const renderToken = (item: ITokenItem, focused: boolean): ReactNode => (
		<ContextMenu key={item.id}>
			<ContextMenuTrigger
				className="contents"
				onContextMenu={(event) => {
					menuPosition.current = { x: event.clientX, y: event.clientY };
				}}
			>
				{item.kind === "function" && item.layer ? (
					<FunctionToken
						layer={item.layer}
						calls={item.uses}
						inputs={pinCount(item.layer, "Input")}
						outputs={pinCount(item.layer, "Output")}
						tint={tint}
						focused={focused}
						onOpen={() => setEditingFunction(item.layer ?? null)}
					/>
				) : item.variable ? (
					<VariableToken
						variable={item.variable}
						uses={item.uses}
						tint={tint}
						focused={focused}
						onOpen={() => setEditingVariable(item.variable ?? null)}
					/>
				) : null}
			</ContextMenuTrigger>
			{renderMenu(item)}
		</ContextMenu>
	);

	const groupModes: Array<{ mode: IGroupMode; label: string }> = [
		{ mode: "folder", label: t("folder", "folder") },
		{ mode: "type", label: t("type", "type") },
		{ mode: "scope", label: t("scope", "scope") },
		{ mode: "usage", label: t("usage", "usage") },
	];

	return (
		<div className="flex h-full flex-col overflow-hidden bg-card">
			<Tabs
				value={tab}
				onValueChange={(value) => setTab(value as "variables" | "functions")}
				className="shrink-0 gap-0"
			>
				<div className="flex items-center gap-1 px-2 pt-2">
					<TabsList className="h-7 bg-transparent p-0">
						<TabsTrigger value="variables" className="h-7 gap-1.5 text-xs">
							{t("variables", "Variables")}
							<span className="font-mono text-[10px] text-muted-foreground">
								{totalVariables}
							</span>
						</TabsTrigger>
						<TabsTrigger value="functions" className="h-7 gap-1.5 text-xs">
							{t("functions", "Functions")}
							<span className="font-mono text-[10px] text-muted-foreground">
								{totalFunctions}
							</span>
						</TabsTrigger>
					</TabsList>
					<div className="flex-1" />
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						aria-pressed={tint}
						title={t("toggleTypeTint", "Type tint on/off")}
						onClick={() => setTint((value) => !value)}
					>
						<ContrastIcon
							className={cn("h-3.5 w-3.5", !tint && "text-muted-foreground")}
						/>
					</Button>
					<TokenLegend />
					{tab === "variables" && currentFunctionLayer ? (
						<DropdownMenu>
							<DropdownMenuTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-7 w-7"
									title={t("newVariable", "New variable")}
								>
									<CirclePlusIcon className="h-3.5 w-3.5" />
								</Button>
							</DropdownMenuTrigger>
							<DropdownMenuContent align="end">
								<DropdownMenuItem
									onClick={() => {
										setNewVariableScope("local");
										setShowNewVariableDialog(true);
									}}
								>
									{t("newLocalVariableIn", "New local variable in {{name}}", {
										name: currentFunctionLayer.name,
									})}
								</DropdownMenuItem>
								<DropdownMenuItem
									onClick={() => {
										setNewVariableScope("board");
										setShowNewVariableDialog(true);
									}}
								>
									{t("newBoardVariable", "New board variable")}
								</DropdownMenuItem>
							</DropdownMenuContent>
						</DropdownMenu>
					) : (
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							title={
								tab === "variables"
									? t("newVariable", "New variable")
									: t("newFunction", "New Function")
							}
							onClick={() => {
								if (tab !== "variables") {
									setDraftFunction("");
									return;
								}
								setNewVariableScope("board");
								setShowNewVariableDialog(true);
							}}
						>
							<CirclePlusIcon className="h-3.5 w-3.5" />
						</Button>
					)}
				</div>
				<TabsContent value="variables" />
				<TabsContent value="functions" />
			</Tabs>

			<div className="flex items-center gap-1.5 px-2 pb-1.5 pt-1.5">
				<div className="flex h-7 flex-1 items-center gap-1.5 rounded-md border bg-background px-2 focus-within:border-primary">
					<SearchIcon className="h-3 w-3 shrink-0 text-muted-foreground" />
					<input
						ref={searchRef}
						value={rawQuery}
						onChange={(event) => setRawQuery(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Escape") {
								setRawQuery("");
								event.stopPropagation();
							}
						}}
						placeholder={t(
							"filterTokensPlaceholder",
							"filter — name, type:string, is:unused, in:state",
						)}
						spellCheck={false}
						aria-label={t("filter", "Filter")}
						className="min-w-0 flex-1 bg-transparent font-mono text-[11px] outline-none placeholder:text-muted-foreground/75"
					/>
					{rawQuery && (
						<button
							type="button"
							aria-label={t("clearFilter", "Clear filter")}
							onClick={() => {
								setRawQuery("");
								searchRef.current?.focus();
							}}
							className="text-muted-foreground hover:text-foreground"
						>
							<XIcon className="h-3 w-3" />
						</button>
					)}
				</div>
			</div>

			<div className="flex flex-wrap items-center gap-1 px-2 pb-2">
				<span className="mr-1 font-mono text-[9px] uppercase tracking-[0.14em] text-muted-foreground">
					{t("group", "Group")}
				</span>
				{groupModes.map(({ mode, label }) => (
					<button
						key={mode}
						type="button"
						aria-pressed={group === mode}
						onClick={() => setGroup(mode)}
						className={cn(
							"rounded border border-transparent px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground transition-colors hover:text-foreground",
							group === mode && "border-border bg-muted text-foreground",
						)}
					>
						{label}
					</button>
				))}
			</div>

			{currentFunctionLayer && (
				<div className="flex items-center gap-2 border-y bg-background px-2 py-1.5 font-mono text-[10px] text-muted-foreground">
					<CornerUpLeftIcon className="h-3 w-3" />
					<span>{t("inside", "inside")}</span>
					<b className="font-medium text-primary">
						{currentFunctionLayer.name}
					</b>
					<span>·</span>
					<span>{t("localsFirst", "locals first")}</span>
				</div>
			)}

			{draftFunction !== null && (
				<div className="flex items-center gap-1.5 border-b bg-background px-2 py-2">
					<SquareFunctionIcon className="h-3.5 w-3.5 shrink-0 text-primary" />
					<Input
						autoFocus
						value={draftFunction}
						onChange={(event) => setDraftFunction(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter") void commitDraftFunction();
							if (event.key === "Escape") setDraftFunction(null);
						}}
						placeholder={t("functionNamePlaceholder", "functionName")}
						className="h-7 font-mono text-xs"
					/>
					<Button size="sm" className="h-7" onClick={commitDraftFunction}>
						{t("create", "Create")}
					</Button>
				</div>
			)}

			<TokenBoard
				items={items}
				kind={tab === "functions" ? FUNCTION_FOLDER_KIND : VARIABLE_FOLDER_KIND}
				lead={
					localItems.length > 0
						? {
								label: t("localToName", "Local · {{name}}", {
									name: currentFunctionLayer?.name ?? "",
								}),
								items: localItems,
							}
						: undefined
				}
				sections={tab === "functions" ? functionSections : undefined}
				group={group}
				query={query}
				renderToken={renderToken}
				empty={
					<div className="px-4 py-6 text-xs leading-relaxed text-muted-foreground">
						{rawQuery ? (
							<>
								{t("nothingMatches", "Nothing matches")}{" "}
								<span className="font-mono text-foreground">{rawQuery}</span>.{" "}
								{t(
									"tryTypeStringIsUnused",
									"Try type:string, is:unused or in:submit.",
								)}
							</>
						) : tab === "functions" ? (
							t("noFunctionsYet", "No functions yet.")
						) : (
							t("noVariablesYet", "No variables yet.")
						)}
					</div>
				}
			/>

			<div className="flex shrink-0 items-center gap-1.5 border-t px-2 py-1.5 font-mono text-[9.5px] text-muted-foreground">
				<span>
					{items.length ===
					(tab === "variables" ? totalVariables : totalFunctions)
						? items.length
						: `${items.length} / ${tab === "variables" ? totalVariables : totalFunctions}`}
				</span>
				{unusedCount > 0 && (
					<>
						<span className="opacity-50">·</span>
						<button
							type="button"
							className="text-destructive hover:underline"
							onClick={() => setRawQuery("is:unused")}
						>
							{t("unusedCount", "{{count}} unused", { count: unusedCount })}
						</button>
					</>
				)}
				<div className="flex-1" />
				<span>
					{currentFunctionLayer
						? t("localScope", "local scope")
						: t("boardScope", "board scope")}
				</span>
			</div>

			<NewVariableDialog
				open={showNewVariableDialog}
				onOpenChange={setShowNewVariableDialog}
				onCreateVariable={
					newVariableScope === "local" && currentFunctionLayer
						? upsertLocalVariable
						: upsertVariable
				}
			/>

			{editingVariable && (
				<VariableOverlay
					key={editingVariable.id}
					open={editingVariable !== null}
					onOpenChange={(open) => {
						if (!open) setEditingVariable(null);
					}}
					variable={editingVariable}
					scope={
						currentFunctionLayer?.variables?.[editingVariable.id]
							? "local"
							: "board"
					}
					uses={usage.variables[editingVariable.id] ?? 0}
					folders={variableFolders}
					refs={board.refs}
					onApply={saveVariable}
					onDelete={(variable, scope) => {
						setEditingVariable(null);
						void deleteVariable(variable, scope);
					}}
				/>
			)}

			{editingFunction && (
				<FunctionOverlay
					open={editingFunction !== null}
					onOpenChange={(open) => {
						if (!open) setEditingFunction(null);
					}}
					layer={editingFunction}
					calls={usage.functions[editingFunction.id] ?? 0}
					folders={functionFolders}
					boardRef={boardRef}
					onApply={async (updated) => {
						await upsertFunction(updated);
						setEditingFunction(null);
					}}
					onDelete={() => {
						const layer = editingFunction;
						setEditingFunction(null);
						void deleteFunction(layer);
					}}
					onOpenLayer={() => {
						const layer = editingFunction;
						setEditingFunction(null);
						void openFunction(layer);
					}}
				/>
			)}
		</div>
	);
}

/** The type vocabulary, one click away instead of learned by osmosis. */
function TokenLegend() {
	const { t } = useTranslation("flow");
	const types = [
		IVariableType.String,
		IVariableType.Integer,
		IVariableType.Float,
		IVariableType.Boolean,
		IVariableType.Struct,
		IVariableType.Date,
		IVariableType.Byte,
		IVariableType.Generic,
		IVariableType.PathBuf,
	];

	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button
					variant="ghost"
					size="icon"
					className="h-7 w-7"
					title={t("legend", "Legend")}
				>
					<HelpCircleIcon className="h-3.5 w-3.5" />
				</Button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-72 text-xs">
				<p className="mb-2 font-mono text-[9.5px] uppercase tracking-[0.14em] text-muted-foreground">
					{t("glyphIsTheType", "Glyph = type")}
				</p>
				<div className="mb-3 grid grid-cols-2 gap-1">
					{types.map((type) => (
						<div key={type} className="flex items-center gap-2">
							<span
								className="flex h-4 w-5 items-center justify-center rounded-[2px] font-mono text-[10px]"
								style={{
									backgroundColor: typeToColor(type),
									color: "var(--card)",
								}}
							>
								{TOKEN_GLYPH[type]}
							</span>
							<span className="text-muted-foreground">{type}</span>
						</div>
					))}
				</div>
				<p className="mb-2 font-mono text-[9.5px] uppercase tracking-[0.14em] text-muted-foreground">
					{t("formIsTheContainer", "Form = container")}
				</p>
				<ul className="mb-3 space-y-1 text-muted-foreground">
					<li>{t("plainSingleValue", "Plain — single value")}</li>
					<li>{t("stackedArray", "Stacked plate — Array")}</li>
					<li>{t("chamferedSet", "Chamfered ends — HashSet")}</li>
					<li>{t("pairedCellsMap", "Paired cells — HashMap")}</li>
					<li>{t("arrowTailFunction", "Arrow tail — callable function")}</li>
					<li>{t("dashedUnused", "Dashed outline — nothing references it")}</li>
				</ul>
				<p className="mb-2 font-mono text-[9.5px] uppercase tracking-[0.14em] text-muted-foreground">
					{t("filterPredicates", "Filter predicates")}
				</p>
				<p className="font-mono text-[10px] leading-relaxed text-muted-foreground">
					type:string · in:state ·{" "}
					{IS_PREDICATES.map((predicate) => `is:${predicate}`).join(" · ")}
				</p>
			</PopoverContent>
		</Popover>
	);
}

export function ValueTypeIcon({
	value_type,
	data_type,
	className,
}: Readonly<{
	value_type: IValueType;
	data_type: IVariableType;
	className?: string;
}>) {
	if (value_type === IValueType.Array)
		return (
			<GripIcon
				className={`w-4 h-4 ${className}`}
				style={{ color: typeToColor(data_type) }}
			/>
		);
	if (value_type === IValueType.HashSet)
		return (
			<EllipsisVerticalIcon
				className={`w-4 h-4 ${className}`}
				style={{ color: typeToColor(data_type) }}
			/>
		);
	if (value_type === IValueType.HashMap)
		return (
			<ListIcon
				className={`w-4 h-4 ${className}`}
				style={{ color: typeToColor(data_type) }}
			/>
		);
	return (
		<div
			className={`w-4 h-2 min-h-2 min-w-4 rounded-full ${className}`}
			style={{ backgroundColor: typeToColor(data_type) }}
		/>
	);
}
