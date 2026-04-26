import {
	useDraggable,
	/* DndContext, type DragEndEvent, PointerSensor, useSensor, useSensors, closestCenter, */ useDroppable,
} from "@dnd-kit/core";
import {
	BracesIcon,
	ChevronDown,
	ChevronRight,
	CircleDotIcon,
	CirclePlusIcon,
	EllipsisVerticalIcon,
	EyeIcon,
	EyeOffIcon,
	FolderIcon,
	GripIcon,
	ListIcon,
	Trash2Icon,
	WandIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { RefObject } from "react";
import { Button } from "../../../components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../../components/ui/dropdown-menu";
import { Input } from "../../../components/ui/input";
import { Label } from "../../../components/ui/label";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
	SelectValue,
} from "../../../components/ui/select";
import { Separator } from "../../../components/ui/separator";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	SheetTrigger,
} from "../../../components/ui/sheet";
import { Switch } from "../../../components/ui/switch";
import {
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../../../components/ui/tabs";
import {
	type IGenericCommand,
	removeVariableCommand,
	upsertVariableCommand,
} from "../../../lib";
import type { IBoard, ILayer, IVariable } from "../../../lib/schema/flow/board";
import { ILayerType } from "../../../lib/schema/flow/board";
import { IVariableType } from "../../../lib/schema/flow/node";
import { IValueType } from "../../../lib/schema/flow/pin";
import { convertJsonToUint8Array } from "../../../lib/uint8";
import { cn } from "../../../lib/utils";
import { FunctionsList, useCreateFunction } from "../functions/functions-menu";
import { typeToColor } from "../utils";
import { NewVariableDialog } from "./new-variable-dialog";
import { VariablesMenuEdit } from "./variables-menu-edit";

export function VariablesMenu({
	board,
	executeCommand,
	currentLayerId,
	pushLayer,
	boardRef,
}: Readonly<{
	board: IBoard;
	executeCommand: (command: IGenericCommand, append: boolean) => Promise<any>;
	currentLayerId?: string;
	pushLayer?: (layer: ILayer) => Promise<void>;
	boardRef?: RefObject<IBoard | undefined>;
}>) {
	const [showNewVariableDialog, setShowNewVariableDialog] = useState(false);
	const [showNewLocalVariableDialog, setShowNewLocalVariableDialog] =
		useState(false);
	const createFunction = useCreateFunction(executeCommand);

	const currentFunctionLayer = useMemo(() => {
		if (!currentLayerId) return null;
		const layer = board.layers[currentLayerId];
		if (!layer || layer.type !== ILayerType.Function) return null;
		return layer;
	}, [currentLayerId, board.layers]);

	const upsertVariable = useCallback(
		async (variable: IVariable) => {
			const oldVariable = board.variables[variable.id];
			if (oldVariable === variable) return;
			const command = upsertVariableCommand({
				variable,
			});

			await executeCommand(command, false);
		},
		[board],
	);

	const upsertLocalVariable = useCallback(
		async (variable: IVariable) => {
			if (!currentFunctionLayer) return;
			const command = upsertVariableCommand({
				variable,
				layer_id: currentFunctionLayer.id,
			});
			await executeCommand(command, false);
		},
		[currentFunctionLayer, executeCommand],
	);

	const removeLocalVariable = useCallback(
		async (variable: IVariable) => {
			if (!currentFunctionLayer) return;
			const command = removeVariableCommand({
				variable,
				layer_id: currentFunctionLayer.id,
			});
			await executeCommand(command, false);
		},
		[currentFunctionLayer, executeCommand],
	);

	const removeVariable = useCallback(
		async (variable: IVariable) => {
			const command = removeVariableCommand({
				variable,
			});
			await executeCommand(command, false);
		},
		[board],
	);

	const tree = useMemo(
		() => buildCategoryTree(Object.values(board.variables)),
		[board.variables],
	);

	const localTree = useMemo(() => {
		if (!currentFunctionLayer) return null;
		return buildCategoryTree(Object.values(currentFunctionLayer.variables));
	}, [currentFunctionLayer]);

	// Listen for cross-folder drops dispatched by FlowWrapper
	useEffect(() => {
		const handler = (e: Event) => {
			const { variable, targetPath } = (e as CustomEvent).detail as {
				variable: IVariable;
				targetPath: string;
			};
			if (!variable?.editable) return;
			const nextCategory = targetPath === "__root" ? undefined : targetPath;
			if (variable.category === nextCategory) return;
			void upsertVariable({ ...variable, category: nextCategory });
		};
		document.addEventListener(
			"variables-folder-drop",
			handler as EventListener,
		);
		return () =>
			document.removeEventListener(
				"variables-folder-drop",
				handler as EventListener,
			);
	}, [upsertVariable]);

	return (
		<div className="flex flex-col h-full overflow-hidden">
			<div className="flex-1 overflow-y-auto overscroll-contain px-4 py-4">
				{!currentFunctionLayer && (
					<div className="flex flex-row items-center gap-4 pb-2 shrink-0">
						<h3 className="text-sm font-semibold text-muted-foreground">
							Variables
						</h3>
						<Button
							variant="ghost"
							size="sm"
							className="gap-1 h-6"
							onClick={() => setShowNewVariableDialog(true)}
						>
							<CirclePlusIcon className="w-3 h-3" />
							New
						</Button>
					</div>
				)}

				<NewVariableDialog
					open={showNewVariableDialog}
					onOpenChange={setShowNewVariableDialog}
					onCreateVariable={upsertVariable}
				/>

				{currentFunctionLayer && (
					<NewVariableDialog
						open={showNewLocalVariableDialog}
						onOpenChange={setShowNewLocalVariableDialog}
						onCreateVariable={upsertLocalVariable}
					/>
				)}

				{currentFunctionLayer && localTree && (
					<>
						<div className="flex flex-row items-center gap-4 pb-2 shrink-0">
							<h3 className="text-xs font-semibold text-muted-foreground">
								Local — {currentFunctionLayer.name}
							</h3>
							<Button
								variant="ghost"
								size="sm"
								className="gap-1 h-6"
								onClick={() => setShowNewLocalVariableDialog(true)}
							>
								<CirclePlusIcon className="w-3 h-3" />
								New
							</Button>
						</div>
						<CategoryTree
							root={localTree}
							refs={board.refs}
							onVariableChange={(variable) => {
								if (!variable.editable) return;
								upsertLocalVariable(variable);
							}}
							onVariableDeleted={(variable) => {
								if (!variable.editable) return;
								removeLocalVariable(variable);
							}}
						/>
						<Separator className="my-3" />
					</>
				)}
				{!currentFunctionLayer && (
					<CategoryTree
						root={tree}
						refs={board.refs}
						onVariableChange={(variable) => {
							if (!variable.editable) return;
							upsertVariable(variable);
						}}
						onVariableDeleted={(variable) => {
							if (!variable.editable) return;
							removeVariable(variable);
						}}
					/>
				)}
				{currentFunctionLayer && (
					<>
						<div className="flex flex-row items-center gap-4 pb-2 shrink-0">
							<h3 className="text-xs font-semibold text-muted-foreground">
								Global
							</h3>
						</div>
						<CategoryTree
							root={tree}
							refs={board.refs}
							onVariableChange={(variable) => {
								if (!variable.editable) return;
								upsertVariable(variable);
							}}
							onVariableDeleted={(variable) => {
								if (!variable.editable) return;
								removeVariable(variable);
							}}
						/>
					</>
				)}

				{pushLayer && (
					<>
						<Separator className="my-3" />
						<div className="flex flex-row items-center gap-4 pb-2 shrink-0">
							<h3 className="text-sm font-semibold text-muted-foreground">
								Functions
							</h3>
							<Button
								variant="ghost"
								size="sm"
								className="gap-1 h-6"
								onClick={createFunction}
							>
								<CirclePlusIcon className="w-3 h-3" />
								New
							</Button>
						</div>
						<FunctionsList
							board={board}
							executeCommand={executeCommand}
							pushLayer={pushLayer}
							boardRef={boardRef}
						/>
					</>
				)}
			</div>
		</div>
	);
}

export function Variable({
	variable,
	onVariableChange,
	onVariableDeleted,
	preview = false,
	refs,
}: Readonly<{
	variable: IVariable;
	onVariableDeleted: (variable: IVariable) => void;
	onVariableChange: (variable: IVariable) => void;
	preview?: boolean;
	refs?: Record<string, string>;
}>) {
	const { attributes, listeners, setNodeRef, transform } = useDraggable({
		id: variable.id,
		data: variable,
	});

	const [localVariable, setLocalVariable] = useState(variable);
	const [openEdit, setOpenEdit] = useState(false);

	const saveVariable = useCallback(() => {
		if (localVariable === variable) return;
		onVariableChange(localVariable);
	}, [localVariable, variable]);

	function defaultValueFromType(
		valueType: IValueType,
		variableType: IVariableType,
	) {
		if (valueType === IValueType.Array) {
			return [];
		}
		if (valueType === IValueType.HashSet) {
			return new Set();
		}
		if (valueType === IValueType.HashMap) {
			return new Map();
		}
		switch (variableType) {
			case IVariableType.Boolean:
				return false;
			case IVariableType.Date:
				return new Date().toISOString();
			case IVariableType.Float:
				return 0.0;
			case IVariableType.Integer:
				return 0;
			case IVariableType.Generic:
				return null;
			case IVariableType.PathBuf:
				return "";
			case IVariableType.String:
				return "";
			case IVariableType.Struct:
				return {};
			case IVariableType.Byte:
				return null;
			case IVariableType.Execution:
				return null;
		}
	}

	const isArrayDropdown = (
		<DropdownMenu>
			<DropdownMenuTrigger>
				<ValueTypeIcon
					value_type={localVariable.value_type}
					data_type={localVariable.data_type}
				/>
			</DropdownMenuTrigger>
			<DropdownMenuContent>
				<DropdownMenuItem
					className="gap-2"
					onClick={(e) => {
						setLocalVariable((old) => ({
							...old,
							value_type: IValueType.Normal,
							default_value: convertJsonToUint8Array(
								defaultValueFromType(IValueType.Normal, old.data_type),
							),
						}));
						e.stopPropagation();
					}}
				>
					<div
						className="w-4 h-2 rounded-full"
						style={{ backgroundColor: typeToColor(localVariable.data_type) }}
					/>{" "}
					Single
				</DropdownMenuItem>
				<DropdownMenuItem
					className="gap-2"
					onClick={(e) => {
						setLocalVariable((old) => ({
							...old,
							value_type: IValueType.Array,
							default_value: convertJsonToUint8Array(
								defaultValueFromType(IValueType.Array, old.data_type),
							),
						}));
						e.stopPropagation();
					}}
				>
					<GripIcon
						className="w-4 h-4"
						style={{ color: typeToColor(localVariable.data_type) }}
					/>{" "}
					Array
				</DropdownMenuItem>
				<DropdownMenuItem
					className="gap-2"
					onClick={(e) => {
						setLocalVariable((old) => ({
							...old,
							value_type: IValueType.HashSet,
							default_value: convertJsonToUint8Array(
								defaultValueFromType(IValueType.HashSet, old.data_type),
							),
						}));
						e.stopPropagation();
					}}
				>
					<EllipsisVerticalIcon
						className="w-4 h-4"
						style={{ color: typeToColor(localVariable.data_type) }}
					/>{" "}
					Set
				</DropdownMenuItem>
				<DropdownMenuItem
					className="gap-2"
					onClick={(e) => {
						setLocalVariable((old) => ({
							...old,
							value_type: IValueType.HashMap,
							default_value: convertJsonToUint8Array(
								defaultValueFromType(IValueType.HashMap, old.data_type),
							),
						}));
						e.stopPropagation();
					}}
				>
					<ListIcon
						className="w-4 h-4"
						style={{ color: typeToColor(localVariable.data_type) }}
					/>{" "}
					Map
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);

	const element = (
		<div
			ref={setNodeRef}
			className={`relative flex w-full flex-row items-center justify-between gap-2 border p-1 px-2 rounded-md bg-card text-card-foreground ${transform ? "opacity-40" : ""} ${!variable.editable ? "text-muted-foreground" : ""}`}
			{...listeners}
			{...attributes}
		>
			<div className="flex flex-row gap-2 items-center" data-no-dnd>
				{isArrayDropdown}
				<p
					className={`text-start line-clamp-2 ${!variable.editable ? "text-muted-foreground" : ""}`}
				>
					{localVariable.name}
				</p>
			</div>
			<div className="flex flex-row items-center gap-2" data-no-dnd>
				<Button
					disabled={!variable.editable}
					variant="ghost"
					size="icon"
					onClick={(event) => {
						event.stopPropagation();
						setLocalVariable((old) => ({ ...old, exposed: !old.exposed }));
					}}
				>
					{localVariable.exposed ? (
						<EyeIcon className="w-4 h-4" />
					) : (
						<EyeOffIcon className="w-4 h-4" />
					)}
				</Button>
			</div>
		</div>
	);

	if (preview) return element;

	const selectPreviewElement = useCallback((type: IVariableType) => {
		return (
			<div className="flex items-center gap-2">
				<div
					className="size-2 rounded-full"
					style={{ backgroundColor: typeToColor(type) }}
				/>
				<span>{type}</span>
			</div>
		);
	}, []);

	const valueTypePreviewElement = useCallback(
		(valueType: IValueType) => {
			const color = typeToColor(localVariable.data_type);
			const iconClass = "w-3.5 h-3.5";

			const getIcon = () => {
				switch (valueType) {
					case IValueType.Normal:
						return <CircleDotIcon className={iconClass} style={{ color }} />;
					case IValueType.Array:
						return <GripIcon className={iconClass} style={{ color }} />;
					case IValueType.HashSet:
						return (
							<EllipsisVerticalIcon className={iconClass} style={{ color }} />
						);
					case IValueType.HashMap:
						return <ListIcon className={iconClass} style={{ color }} />;
					default:
						return null;
				}
			};

			const getLabel = () => {
				switch (valueType) {
					case IValueType.Normal:
						return "Single";
					case IValueType.Array:
						return "Array";
					case IValueType.HashSet:
						return "Set";
					case IValueType.HashMap:
						return "Map";
					default:
						return valueType;
				}
			};

			return (
				<div className="flex items-center gap-2">
					{getIcon()}
					<span>{getLabel()}</span>
				</div>
			);
		},
		[localVariable.data_type],
	);

	return (
		<Sheet
			open={openEdit}
			onOpenChange={(open) => {
				if (!localVariable.editable) return;
				setOpenEdit(open);
				if (!open) {
					saveVariable();
				}
			}}
		>
			<SheetTrigger asChild>{element}</SheetTrigger>
			<SheetContent className="flex flex-col max-h-screen overflow-hidden px-3 pt-2 pb-4">
				<SheetHeader className="shrink-0">
					<SheetTitle className="flex flex-row items-center gap-2">
						Edit Variable
					</SheetTitle>
					<SheetDescription className="flex flex-col gap-6 text-foreground">
						<p className="text-muted-foreground">
							Edit the variable properties to your liking.
						</p>
					</SheetDescription>
				</SheetHeader>

				<div className="flex-1 overflow-y-auto space-y-6 pr-2">
					<div className="grid w-full max-w-sm items-center gap-1.5">
						<Label htmlFor="name">Variable Name</Label>
						<Input
							value={localVariable.name}
							onChange={(e) => {
								setLocalVariable((old) => ({ ...old, name: e.target.value }));
							}}
							id="name"
							placeholder="Name"
						/>
					</div>

					<div className="grid w-full max-w-sm items-center gap-1.5">
						<Label htmlFor="category">Category</Label>
						<Input
							id="category"
							value={localVariable.category ?? ""}
							onChange={(e) => {
								const v = e.target.value;
								setLocalVariable((old) => ({
									...old,
									category: v.trim() === "" ? undefined : v,
								}));
							}}
							placeholder="e.g. Main/Bools"
						/>
						<small className="text-[0.8rem] text-muted-foreground">
							Use “/” to create nested folders. Leave empty for top-level.
						</small>
					</div>

					<div className="grid w-full max-w-sm items-center gap-1.5">
						<Label htmlFor="var_type">Variable Type</Label>
						<div className="flex flex-row gap-2">
							<Select
								value={localVariable.data_type}
								onValueChange={(value) =>
									setLocalVariable((old) => ({
										...old,
										data_type: value as IVariableType,
										default_value: convertJsonToUint8Array(
											defaultValueFromType(
												old.value_type,
												value as IVariableType,
											),
										),
									}))
								}
							>
								<SelectTrigger id="var_type" className="flex-1">
									<SelectValue placeholder="Data Type" />
								</SelectTrigger>
								<SelectContent>
									<SelectGroup>
										<SelectLabel>Data Type</SelectLabel>
										<SelectItem value="Boolean">
											{selectPreviewElement(IVariableType.Boolean)}
										</SelectItem>
										<SelectItem value="Date">
											{selectPreviewElement(IVariableType.Date)}
										</SelectItem>
										<SelectItem value="Float">
											{selectPreviewElement(IVariableType.Float)}
										</SelectItem>
										<SelectItem value="Integer">
											{selectPreviewElement(IVariableType.Integer)}
										</SelectItem>
										<SelectItem value="Generic">
											{selectPreviewElement(IVariableType.Generic)}
										</SelectItem>
										<SelectItem value="PathBuf">
											{selectPreviewElement(IVariableType.PathBuf)}
										</SelectItem>
										<SelectItem value="String">
											{selectPreviewElement(IVariableType.String)}
										</SelectItem>
										<SelectItem value="Struct">
											{selectPreviewElement(IVariableType.Struct)}
										</SelectItem>
										<SelectItem value="Byte">
											{selectPreviewElement(IVariableType.Byte)}
										</SelectItem>
									</SelectGroup>
								</SelectContent>
							</Select>
							<Select
								value={localVariable.value_type}
								onValueChange={(value) =>
									setLocalVariable((old) => ({
										...old,
										value_type: value as IValueType,
										default_value: convertJsonToUint8Array(
											defaultValueFromType(value as IValueType, old.data_type),
										),
									}))
								}
							>
								<SelectTrigger className="w-28">
									<SelectValue placeholder="Value Type" />
								</SelectTrigger>
								<SelectContent>
									<SelectGroup>
										<SelectLabel>Value Type</SelectLabel>
										<SelectItem value="Normal">
											{valueTypePreviewElement(IValueType.Normal)}
										</SelectItem>
										<SelectItem value="Array">
											{valueTypePreviewElement(IValueType.Array)}
										</SelectItem>
										<SelectItem value="HashSet">
											{valueTypePreviewElement(IValueType.HashSet)}
										</SelectItem>
										<SelectItem value="HashMap">
											{valueTypePreviewElement(IValueType.HashMap)}
										</SelectItem>
									</SelectGroup>
								</SelectContent>
							</Select>
						</div>
					</div>

					<div className="flex flex-col gap-1">
						<div className="flex items-center space-x-2">
							<Switch
								checked={localVariable.exposed}
								onCheckedChange={(checked) =>
									setLocalVariable((old) => ({ ...old, exposed: checked }))
								}
								id="exposed"
							/>
							<Label htmlFor="exposed">Is Exposed?</Label>
						</div>
						<small className="text-[0.8rem] text-muted-foreground">
							If you expose a variable it will be visible in the configuration
							tab of your App.
						</small>
					</div>

					<div className="flex flex-col gap-1">
						<div className="flex items-center space-x-2">
							<Switch
								checked={localVariable.secret}
								onCheckedChange={(checked) =>
									setLocalVariable((old) => ({ ...old, secret: checked }))
								}
								id="secret"
							/>
							<Label htmlFor="secret">Is Secret?</Label>
						</div>
						<small className="text-[0.8rem] text-muted-foreground">
							A secret variable will be covered for input (e.g passwords)
						</small>
					</div>

					<div className="flex flex-col gap-1">
						<div className="flex items-center space-x-2">
							<Switch
								checked={localVariable.runtime_configured ?? false}
								onCheckedChange={(checked) =>
									setLocalVariable((old) => ({
										...old,
										runtime_configured: checked,
									}))
								}
								id="runtime_configured"
							/>
							<Label htmlFor="runtime_configured">Runtime Configured?</Label>
						</div>
						<small className="text-[0.8rem] text-muted-foreground">
							Runtime configured variables are set per-user locally. They are
							never stored in the flow itself.
						</small>
					</div>

					{localVariable.data_type === IVariableType.Struct && (
						<StructSchemaEditor
							variable={localVariable}
							refs={refs}
							onSchemaChange={(schema) =>
								setLocalVariable((old) => ({ ...old, schema }))
							}
						/>
					)}

					<Separator />

					<div className="flex flex-col">
						{!localVariable.exposed && (
							<VariablesMenuEdit
								key={`${localVariable.value_type} - ${localVariable.data_type}-${localVariable.secret}`}
								variable={localVariable}
								refs={refs}
								updateVariable={async (variable) =>
									setLocalVariable((old) => ({
										...old,
										default_value: variable.default_value,
									}))
								}
							/>
						)}
					</div>
				</div>

				<Button
					className="shrink-0"
					variant="destructive"
					onClick={() => {
						onVariableDeleted(variable);
					}}
				>
					<Trash2Icon />
				</Button>
			</SheetContent>
		</Sheet>
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

// Category grouping

type CategoryNode = {
	name: string;
	path: string;
	vars: IVariable[];
	children: Record<string, CategoryNode>;
};

const buildCategoryTree = (vars: IVariable[]): CategoryNode => {
	const root: CategoryNode = { name: "", path: "", vars: [], children: {} };
	for (const v of vars) {
		const raw = (v as any)?.category as string | null | undefined;
		const cat = (raw ?? "").trim();
		if (!cat) {
			root.vars.push(v);
			continue;
		}
		const segments = cat
			.split("/")
			.map((s) => s.trim())
			.filter(Boolean);
		let node = root;
		let path = "";
		for (const seg of segments) {
			path = path ? `${path}/${seg}` : seg;
			if (!node.children[seg])
				node.children[seg] = { name: seg, path, vars: [], children: {} };
			node = node.children[seg];
		}
		node.vars.push(v);
	}
	return root;
};

const countRecursive = (node: CategoryNode): number =>
	node.vars.length +
	Object.values(node.children).reduce((sum, c) => sum + countRecursive(c), 0);

const CategoryTree: React.FC<{
	root: CategoryNode;
	refs?: Record<string, string>;
	onVariableChange: (v: IVariable) => void;
	onVariableDeleted: (v: IVariable) => void;
}> = ({ root, refs, onVariableChange, onVariableDeleted }) => {
	const [open, setOpen] = useState<Record<string, boolean>>({});
	const isOpen = useCallback((path: string) => open[path] ?? true, [open]);
	const toggle = useCallback((path: string) => {
		setOpen((prev) => ({ ...prev, [path]: !(prev[path] ?? true) }));
	}, []);

	// Root droppable to move items to top-level (category undefined)
	const { setNodeRef: setRootRef, isOver: rootOver } = useDroppable({
		id: "__root",
	});

	const childKeys = useMemo(
		() => Object.keys(root.children).sort((a, b) => a.localeCompare(b)),
		[root.children],
	);
	const varsSorted = useMemo(
		() => [...root.vars].sort((a, b) => a.name.localeCompare(b.name)),
		[root.vars],
	);

	return (
		<div
			ref={setRootRef}
			className={`space-y-2 rounded-md ${rootOver ? "ring-1 ring-primary/40" : ""}`}
		>
			{varsSorted.length > 0 && (
				<div className="flex flex-col gap-2">
					{varsSorted.map((variable) => (
						<Variable
							key={variable.id}
							variable={variable}
							refs={refs}
							onVariableChange={onVariableChange}
							onVariableDeleted={onVariableDeleted}
						/>
					))}
				</div>
			)}
			{childKeys.length > 0 && (
				<div className="space-y-2">
					{childKeys.map((k) => (
						<FolderNode
							key={root.children[k].path}
							node={root.children[k]}
							depth={1}
							isOpen={isOpen}
							toggle={toggle}
							refs={refs}
							onVariableChange={onVariableChange}
							onVariableDeleted={onVariableDeleted}
						/>
					))}
				</div>
			)}
		</div>
	);
};

const FolderNode: React.FC<{
	node: CategoryNode;
	depth: number;
	isOpen: (path: string) => boolean;
	toggle: (path: string) => void;
	refs?: Record<string, string>;
	onVariableChange: (v: IVariable) => void;
	onVariableDeleted: (v: IVariable) => void;
}> = ({
	node,
	depth,
	isOpen,
	toggle,
	refs,
	onVariableChange,
	onVariableDeleted,
}) => {
	const { setNodeRef, isOver } = useDroppable({ id: node.path });
	const childKeys = useMemo(
		() => Object.keys(node.children).sort((a, b) => a.localeCompare(b)),
		[node.children],
	);
	const varsSorted = useMemo(
		() => [...node.vars].sort((a, b) => a.name.localeCompare(b.name)),
		[node.vars],
	);
	const total = countRecursive(node);

	return (
		<div className="rounded-md border">
			<button
				ref={setNodeRef}
				type="button"
				className={`w-full flex items-center gap-2 px-2 py-2 hover:bg-accent/50 ${isOver ? "bg-primary/5" : ""}`}
				onClick={() => toggle(node.path)}
			>
				{isOpen(node.path) ? (
					<ChevronDown className="h-4 w-4 text-muted-foreground" />
				) : (
					<ChevronRight className="h-4 w-4 text-muted-foreground" />
				)}
				<FolderIcon className="h-4 w-4 text-muted-foreground" />
				<span className="text-sm font-medium">{node.name}</span>
				<span className="ml-auto text-xs text-muted-foreground">{total}</span>
			</button>

			{isOpen(node.path) && (
				<div className="p-2 pt-1 space-y-2">
					{varsSorted.map((variable) => (
						<Variable
							key={variable.id}
							variable={variable}
							refs={refs}
							onVariableChange={onVariableChange}
							onVariableDeleted={onVariableDeleted}
						/>
					))}
					{childKeys.length > 0 && (
						<div className="mt-2 space-y-2">
							{childKeys.map((k) => (
								<FolderNode
									key={node.children[k].path}
									node={node.children[k]}
									depth={depth + 1}
									isOpen={isOpen}
									toggle={toggle}
									refs={refs}
									onVariableChange={onVariableChange}
									onVariableDeleted={onVariableDeleted}
								/>
							))}
						</div>
					)}
				</div>
			)}
		</div>
	);
};
const EMPTY_STRING_HASH = "16248035215404677707";

const resolveRef = (
	value: string | undefined | null,
	refs: Record<string, string> | undefined,
): string => {
	if (!value) return "";
	if (value === EMPTY_STRING_HASH) return "";
	const resolved = refs?.[value];
	return resolved ?? value;
};

function StructSchemaEditor({
	variable,
	refs,
	onSchemaChange,
}: Readonly<{
	variable: IVariable;
	refs?: Record<string, string>;
	onSchemaChange: (schema: string | null) => void;
}>) {
	const resolvedSchema = useMemo(() => {
		if (!variable.schema) return "";
		return resolveRef(variable.schema, refs);
	}, [variable.schema, refs]);

	const [schemaMode, setSchemaMode] = useState<"example" | "schema">("example");
	const [exampleJson, setExampleJson] = useState("{}");
	const [schemaJson, setSchemaJson] = useState(resolvedSchema || "");
	const [error, setError] = useState<string | null>(null);
	const [isFocused, setIsFocused] = useState(false);

	useEffect(() => {
		if (resolvedSchema) {
			setSchemaJson(resolvedSchema);
		}
	}, [resolvedSchema]);

	const handleGenerateFromExample = useCallback(() => {
		try {
			const parsed = JSON.parse(exampleJson);
			// Generate a simple schema from the example
			const schema = generateSchemaFromExample(parsed);
			const schemaStr = JSON.stringify(schema, null, 2);
			setSchemaJson(schemaStr);
			onSchemaChange(schemaStr);
			setError(null);
		} catch (e) {
			setError("Invalid JSON example");
		}
	}, [exampleJson, onSchemaChange]);

	const handleSchemaChange = useCallback(
		(value: string) => {
			setSchemaJson(value);
			if (!value.trim()) {
				onSchemaChange(null);
				setError(null);
				return;
			}
			try {
				JSON.parse(value);
				onSchemaChange(value);
				setError(null);
			} catch {
				setError("Invalid JSON schema");
			}
		},
		[onSchemaChange],
	);

	return (
		<div className="flex flex-col gap-2">
			<Label className="flex items-center gap-2">
				<BracesIcon className="w-4 h-4" />
				Schema
			</Label>
			<small className="text-[0.8rem] text-muted-foreground -mt-1">
				Define a JSON schema to enable form-based editing for this struct.
			</small>

			<Tabs
				value={schemaMode}
				onValueChange={(v) => setSchemaMode(v as "example" | "schema")}
			>
				<TabsList className="grid w-full grid-cols-2">
					<TabsTrigger value="example" className="gap-1">
						<WandIcon className="w-3 h-3" />
						From Example
					</TabsTrigger>
					<TabsTrigger value="schema" className="gap-1">
						<BracesIcon className="w-3 h-3" />
						Edit Schema
					</TabsTrigger>
				</TabsList>

				<TabsContent value="example" className="space-y-2">
					<small className="text-[0.8rem] text-muted-foreground">
						Paste an example JSON and generate a schema automatically.
					</small>
					<div
						className={cn(
							"relative w-full rounded-md border bg-transparent transition-all duration-200",
							"border-input dark:bg-input/30",
							isFocused && "border-ring ring-ring/50 ring-[3px]",
						)}
					>
						<textarea
							autoComplete="off"
							autoCorrect="off"
							autoCapitalize="off"
							value={exampleJson}
							onChange={(e) => setExampleJson(e.target.value)}
							onFocus={() => setIsFocused(true)}
							onBlur={() => setIsFocused(false)}
							placeholder='{"name": "John", "age": 30}'
							rows={5}
							className="w-full resize-none bg-transparent px-3 py-2 text-sm outline-none font-mono"
						/>
					</div>
					<Button
						type="button"
						variant="secondary"
						size="sm"
						className="gap-1"
						onClick={handleGenerateFromExample}
					>
						<WandIcon className="w-3 h-3" />
						Generate Schema
					</Button>
				</TabsContent>

				<TabsContent value="schema" className="space-y-2">
					<small className="text-[0.8rem] text-muted-foreground">
						Edit the JSON schema directly. Leave empty to disable form mode.
					</small>
					<div
						className={cn(
							"relative w-full rounded-md border bg-transparent transition-all duration-200",
							"border-input dark:bg-input/30",
							isFocused && "border-ring ring-ring/50 ring-[3px]",
							error && "border-destructive",
						)}
					>
						<textarea
							value={schemaJson}
							onChange={(e) => handleSchemaChange(e.target.value)}
							onFocus={() => setIsFocused(true)}
							onBlur={() => setIsFocused(false)}
							placeholder='{"type": "object", "properties": {...}}'
							rows={8}
							className="w-full resize-none bg-transparent px-3 py-2 text-sm outline-none font-mono"
						/>
					</div>
					{error && <p className="text-xs text-destructive">{error}</p>}
					{schemaJson && !error && (
						<Button
							type="button"
							variant="outline"
							size="sm"
							onClick={() => handleSchemaChange("")}
						>
							Clear Schema
						</Button>
					)}
				</TabsContent>
			</Tabs>
		</div>
	);
}

function generateSchemaFromExample(example: unknown): object {
	if (example === null) {
		return { type: "null" };
	}

	if (Array.isArray(example)) {
		const itemSchema =
			example.length > 0 ? generateSchemaFromExample(example[0]) : {};
		return { type: "array", items: itemSchema };
	}

	if (typeof example === "object") {
		const properties: Record<string, object> = {};
		const required: string[] = [];

		for (const [key, value] of Object.entries(example)) {
			properties[key] = generateSchemaFromExample(value);
			if (value !== null && value !== undefined) {
				required.push(key);
			}
		}

		return {
			type: "object",
			properties,
			required: required.length > 0 ? required : undefined,
		};
	}

	if (typeof example === "boolean") {
		return { type: "boolean" };
	}

	if (typeof example === "number") {
		return Number.isInteger(example) ? { type: "integer" } : { type: "number" };
	}

	if (typeof example === "string") {
		return { type: "string" };
	}

	return {};
}
