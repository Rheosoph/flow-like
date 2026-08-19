"use client";

import {
	DndContext,
	type DragEndEvent,
	PointerSensor,
	closestCenter,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	SortableContext,
	arrayMove,
	useSortable,
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	Boxes,
	Database,
	GripVertical,
	Loader2,
	Plus,
	Trash2,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { cn } from "../../../lib";
import { useBackend } from "../../../state/backend-state";
import type { IDatabaseSchemaField } from "../../../state/backend-state/db-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Switch } from "../../ui/switch";
import {
	ColumnTypeSelect,
	IndexTypeSelect,
	NullableSelect,
	indexTypeEnum,
	validateColumnName,
	validateTableName,
} from "../../ui/table-schema";

interface ColumnDraft {
	id: string;
	name: string;
	type: string;
	nullable: boolean;
	vectorSize: string;
	indexed: boolean;
	indexType: string;
}

function newColumn(): ColumnDraft {
	return {
		id: createId(),
		name: "",
		type: "string",
		nullable: true,
		vectorSize: "",
		indexed: false,
		indexType: "auto",
	};
}

export interface TableDesignerDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	appId: string;
	existingTables?: string[];
	onCreated: (tableName: string, userScoped: boolean) => void;
}

export function TableDesignerDialog({
	open,
	onOpenChange,
	appId,
	existingTables,
	onCreated,
}: Readonly<TableDesignerDialogProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [tableName, setTableName] = useState("");
	const [scope, setScope] = useState<"project" | "user">("project");
	const [columns, setColumns] = useState<ColumnDraft[]>(() => [newColumn()]);
	const [submitting, setSubmitting] = useState(false);

	const sensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
	);

	const reset = useCallback(() => {
		setTableName("");
		setScope("project");
		setColumns([newColumn()]);
		setSubmitting(false);
	}, []);

	const handleOpenChange = useCallback(
		(next: boolean) => {
			if (submitting) return;
			if (!next) reset();
			onOpenChange(next);
		},
		[onOpenChange, reset, submitting],
	);

	const updateColumn = useCallback(
		(id: string, patch: Partial<ColumnDraft>) => {
			setColumns((prev) =>
				prev.map((column) =>
					column.id === id ? { ...column, ...patch } : column,
				),
			);
		},
		[],
	);

	const removeColumn = useCallback((id: string) => {
		setColumns((prev) =>
			prev.length > 1 ? prev.filter((column) => column.id !== id) : prev,
		);
	}, []);

	const addColumn = useCallback(() => {
		setColumns((prev) => [...prev, newColumn()]);
	}, []);

	const handleDragEnd = useCallback((event: DragEndEvent) => {
		const { active, over } = event;
		if (!over || active.id === over.id) return;
		setColumns((prev) => {
			const from = prev.findIndex((column) => column.id === active.id);
			const to = prev.findIndex((column) => column.id === over.id);
			if (from < 0 || to < 0) return prev;
			return arrayMove(prev, from, to);
		});
	}, []);

	const tableNameError = useMemo(() => {
		const error = validateTableName(tableName);
		if (error) return tableName.trim() ? error : null;
		const exists = existingTables?.some(
			(name) => name.toLowerCase() === tableName.trim().toLowerCase(),
		);
		return exists
			? t(
					"aTableWithThisNameAlreadyExists",
					"A table with this name already exists",
				)
			: null;
	}, [tableName, existingTables]);

	const duplicateNames = useMemo(() => {
		const seen = new Map<string, number>();
		for (const column of columns) {
			const key = column.name.trim().toLowerCase();
			if (!key) continue;
			seen.set(key, (seen.get(key) ?? 0) + 1);
		}
		return new Set(
			[...seen.entries()].filter(([, count]) => count > 1).map(([key]) => key),
		);
	}, [columns]);

	const columnError = useCallback(
		(column: ColumnDraft): string | null => {
			const nameError = validateColumnName(column.name);
			if (nameError) return column.name.trim() ? nameError : null;
			if (duplicateNames.has(column.name.trim().toLowerCase()))
				return t("duplicateColumnName", "Duplicate column name");
			if (column.type === "vector") {
				const size = Number.parseInt(column.vectorSize, 10);
				if (!Number.isFinite(size) || size <= 0)
					return t(
						"vectorSizeMustBeAPositiveNumber",
						"Vector size must be a positive number",
					);
			}
			return null;
		},
		[duplicateNames],
	);

	const canSubmit = useMemo(() => {
		if (validateTableName(tableName)) return false;
		if (tableNameError) return false;
		if (!columns.length) return false;
		return columns.every(
			(column) => column.name.trim() && columnError(column) === null,
		);
	}, [tableName, tableNameError, columns, columnError]);

	const handleCreate = useCallback(async () => {
		if (!canSubmit) return;
		const userScoped = scope === "user";
		const name = tableName.trim();
		const fields: IDatabaseSchemaField[] = columns.map((column) => ({
			name: column.name.trim(),
			type: column.type,
			nullable: column.nullable,
			...(column.type === "vector"
				? { vector_size: Number.parseInt(column.vectorSize, 10) }
				: {}),
		}));
		const indexed = columns.filter(
			(column) => column.indexed && column.type !== "vector",
		);

		setSubmitting(true);
		try {
			await backend.dbState.createTable(appId, name, fields, false, userScoped);
		} catch (error) {
			toast.error(
				t("failedToCreateTableVal", "Failed to create table: {{val}}", {
					val: error instanceof Error ? error.message : String(error),
				}),
			);
			setSubmitting(false);
			return;
		}

		const failedIndexes: string[] = [];
		for (const column of indexed) {
			try {
				await backend.dbState.buildIndex(
					appId,
					name,
					column.name.trim(),
					indexTypeEnum(column.indexType),
					undefined,
					userScoped,
				);
			} catch {
				failedIndexes.push(column.name.trim());
			}
		}

		if (failedIndexes.length) {
			toast.success(
				t(
					"createdTableNameIndexOnValWillBuildOnceTheTableHasData",
					'Created table "{{name}}". Index on {{val}} will build once the table has data.',
					{ name, val: failedIndexes.join(", ") },
				),
			);
		} else {
			toast.success(`Created table "${name}"`);
		}

		reset();
		onOpenChange(false);
		onCreated(name, userScoped);
	}, [
		appId,
		backend.dbState,
		canSubmit,
		columns,
		onCreated,
		onOpenChange,
		reset,
		scope,
		tableName,
	]);

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="max-w-3xl max-h-[85vh] overflow-hidden flex flex-col">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Database className="h-5 w-5 text-primary" />{" "}
						{t("createTable", "Create table")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"designANativeTableWithTypedColumnsNullabilityAndIndexesDragToReorder",
							"Design a native table with typed columns, nullability, and indexes. Drag to reorder.",
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="grid gap-4 shrink-0 sm:grid-cols-[1fr_200px]">
					<div className="space-y-1.5">
						<Label htmlFor="table-designer-name">
							{t("tableName", "Table name")}
						</Label>
						<Input
							id="table-designer-name"
							value={tableName}
							onChange={(event) => setTableName(event.target.value)}
							placeholder="customers"
							autoComplete="off"
						/>
						{tableNameError && (
							<p className="text-xs text-destructive">{tableNameError}</p>
						)}
					</div>
					<div className="space-y-1.5">
						<Label htmlFor="table-designer-scope">{t("scope", "Scope")}</Label>
						<Select
							value={scope}
							onValueChange={(value) => setScope(value as "project" | "user")}
						>
							<SelectTrigger id="table-designer-scope">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="project">
									{t("projectShared", "Project (shared)")}
								</SelectItem>
								<SelectItem value="user">
									{t("userScoped", "User scoped")}
								</SelectItem>
							</SelectContent>
						</Select>
					</div>
				</div>

				<div className="flex shrink-0 items-center justify-between">
					<Label>{t("columns", "Columns")}</Label>
					<Button variant="outline" size="sm" onClick={addColumn}>
						<Plus className="h-4 w-4" /> {t("addColumn", "Add column")}
					</Button>
				</div>

				<DialogBody className="-mx-1 px-1">
					<DndContext
						sensors={sensors}
						collisionDetection={closestCenter}
						onDragEnd={handleDragEnd}
					>
						<SortableContext
							items={columns.map((column) => column.id)}
							strategy={verticalListSortingStrategy}
						>
							<div className="space-y-2">
								{columns.map((column) => (
									<ColumnDesignerRow
										key={column.id}
										column={column}
										error={columnError(column)}
										canRemove={columns.length > 1}
										onChange={(patch) => updateColumn(column.id, patch)}
										onRemove={() => removeColumn(column.id)}
									/>
								))}
							</div>
						</SortableContext>
					</DndContext>
				</DialogBody>

				<DialogFooter>
					<Button
						variant="ghost"
						onClick={() => handleOpenChange(false)}
						disabled={submitting}
					>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={handleCreate} disabled={!canSubmit || submitting}>
						{submitting ? (
							<Loader2 className="h-4 w-4 animate-spin" />
						) : (
							<Plus className="h-4 w-4" />
						)}
						{t("createTable", "Create table")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function ColumnDesignerRow({
	column,
	error,
	canRemove,
	onChange,
	onRemove,
}: Readonly<{
	column: ColumnDraft;
	error: string | null;
	canRemove: boolean;
	onChange: (patch: Partial<ColumnDraft>) => void;
	onRemove: () => void;
}>) {
	const { t } = useTranslation("settings");
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({ id: column.id });
	const isVector = column.type === "vector";

	return (
		<div
			ref={setNodeRef}
			style={{
				transform: CSS.Transform.toString(transform),
				transition,
			}}
			className={cn(
				"rounded-lg border bg-card p-3",
				isDragging && "opacity-60 shadow-lg",
			)}
		>
			<div className="flex items-start gap-2">
				<button
					type="button"
					className="mt-2 cursor-grab text-muted-foreground hover:text-foreground touch-none"
					aria-label={t("reorderColumn", "Reorder column")}
					{...attributes}
					{...listeners}
				>
					<GripVertical className="h-4 w-4" />
				</button>
				<div className="grid flex-1 gap-2 sm:grid-cols-[1fr_1fr_130px]">
					<Input
						value={column.name}
						onChange={(event) => onChange({ name: event.target.value })}
						placeholder={t("columnName", "Column name")}
						autoComplete="off"
						aria-invalid={error ? true : undefined}
					/>
					<ColumnTypeSelect
						value={column.type}
						onChange={(type) =>
							onChange({
								type,
								...(type === "vector" ? {} : { vectorSize: "" }),
							})
						}
					/>
					<NullableSelect
						nullable={column.nullable}
						onChange={(nullable) => onChange({ nullable })}
					/>
				</div>
				<Button
					variant="ghost"
					size="icon"
					className="h-9 w-9 shrink-0 text-muted-foreground hover:text-destructive"
					onClick={onRemove}
					disabled={!canRemove}
					aria-label={t("removeColumn", "Remove column")}
				>
					<Trash2 className="h-4 w-4" />
				</Button>
			</div>

			{isVector && (
				<div className="mt-2 flex items-center gap-2 pl-6">
					<Boxes className="h-4 w-4 text-muted-foreground" />
					<Label className="text-xs text-muted-foreground">
						{t("dimensions", "Dimensions")}
					</Label>
					<Input
						type="number"
						min={1}
						value={column.vectorSize}
						onChange={(event) => onChange({ vectorSize: event.target.value })}
						placeholder="1536"
						className="h-8 w-32"
					/>
				</div>
			)}

			<div className="mt-2 flex flex-wrap items-center gap-2 pl-6">
				<Switch
					id={`index-${column.id}`}
					checked={column.indexed}
					onCheckedChange={(indexed) => onChange({ indexed })}
					disabled={isVector}
				/>
				<Label
					htmlFor={`index-${column.id}`}
					className="text-xs text-muted-foreground"
				>
					{t("index", "Index")}
				</Label>
				{column.indexed && !isVector && (
					<IndexTypeSelect
						value={column.indexType}
						onChange={(indexType) => onChange({ indexType })}
						className="h-8 w-36"
					/>
				)}
				{isVector && (
					<Badge variant="secondary" className="text-[10px]">
						{t(
							"vectorColumnsAreIndexedSeparately",
							"Vector columns are indexed separately",
						)}
					</Badge>
				)}
			</div>

			{error && <p className="mt-2 pl-6 text-xs text-destructive">{error}</p>}
		</div>
	);
}
