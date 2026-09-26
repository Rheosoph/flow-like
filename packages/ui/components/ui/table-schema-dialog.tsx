"use client";

import { useTranslation } from "@flow-like/locales";
import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import {
	AlertTriangle,
	ChevronLeft,
	Database,
	History,
	KeyRound,
	Loader2,
	Lock,
	Plus,
	Search,
	Settings,
	Trash2,
	Zap,
} from "lucide-react";
import type * as React from "react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { cn } from "../../lib";
import { asArray } from "../../lib/response-shape";
import {
	type IIndexConfig,
	IIndexType,
	parseIndexType,
} from "../../state/backend-state/db-state";
import { Badge } from "./badge";
import { Button } from "./button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "./dialog";
import { Input } from "./input";
import { Label } from "./label";
import type { LanceField, LanceSchema } from "./lance-viewer";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "./table";
import {
	EDIT_COLUMN_TYPE_GROUPS,
	INDEX_TYPE_OPTIONS,
	IndexTypeHelp,
	IndexTypeSelect,
	addColumnSqlType,
	buildAddColumnExpression,
	validateColumnDefault,
	validateColumnName,
} from "./table-schema";

export interface TableSchemaDialogProps {
	schema: LanceSchema | null;
	tableName?: string;
	rowCount?: number;
	onDropColumns?: (columns: string[]) => Promise<void>;
	onAddColumn?: (name: string, sqlExpression: string) => Promise<void>;
	onAlterColumn?: (column: string, nullable: boolean) => Promise<void>;
	onSetPrimaryKey?: (column: string) => Promise<void>;
	onBuildIndex?: (column: string, indexType: string) => Promise<void>;
	onGetIndices?: () => Promise<IIndexConfig[]>;
	onDropIndex?: (indexName: string) => Promise<void>;
}

type SchemaPane = "column" | "new" | "indices";
type IndexStatus = "idle" | "loading" | "ready" | "error";
type ColumnConfirm = "nullable" | "key" | "drop";
type ColumnAction = ColumnConfirm | "required" | "buildIndex" | "dropIndex";

export const describeField = (f: LanceField): string => {
	switch (f.kind) {
		case "vector":
			return `${f.kind}${f.dims ? `(${f.dims})` : ""}`;
		case "array":
			return `array<${
				typeof f.items === "string" ? f.items : (f.items?.kind ?? "unknown")
			}>`;
		default:
			return f.kind;
	}
};

const TEMPORAL_SUFFIX: Record<string, string> = {
	second: "s",
	millisecond: "ms",
	microsecond: "us",
	nanosecond: "ns",
};

export function fieldTypeLabel(field: LanceField): string {
	const suffix =
		field.kind === "date" && field.temporal
			? TEMPORAL_SUFFIX[field.temporal]
			: undefined;
	return suffix ? `timestamp(${suffix})` : describeField(field);
}

function columnIndex(indices: readonly IIndexConfig[], column: string) {
	return indices.find((index) => index.columns.includes(column));
}

function indexOption(index: IIndexConfig) {
	const type = parseIndexType(index.index_type);
	if (type === IIndexType.Auto) return undefined;
	return INDEX_TYPE_OPTIONS.find((option) => option.type === type);
}

const indexLabel = (index: IIndexConfig) =>
	indexOption(index)?.label ?? index.index_type;

/** Hosts toast their own failures and rethrow; the dialog only needs the outcome. */
async function attempt(task: () => Promise<void>): Promise<boolean> {
	try {
		await task();
		return true;
	} catch {
		return false;
	}
}

function useTableIndices(onGetIndices?: () => Promise<IIndexConfig[]>) {
	const [indices, setIndices] = useState<IIndexConfig[]>([]);
	const [status, setStatus] = useState<IndexStatus>("idle");
	const request = useRef(0);

	const reload = useCallback(async () => {
		if (!onGetIndices) return;
		const id = ++request.current;
		setStatus("loading");
		try {
			const next = await onGetIndices();
			if (id !== request.current) return;
			setIndices(asArray(next));
			setStatus("ready");
		} catch {
			if (id === request.current) setStatus("error");
		}
	}, [onGetIndices]);

	useEffect(() => {
		void reload();
	}, [reload]);

	return { indices, status, reload };
}

export function TableSchemaDialog(props: Readonly<TableSchemaDialogProps>) {
	const { t } = useTranslation("common");
	const [open, setOpen] = useState(false);
	const contentRef = useRef<HTMLDivElement>(null);
	return (
		<>
			<Button variant="outline" size="sm" onClick={() => setOpen(true)}>
				<Settings /> {t("schema", "Schema")}
			</Button>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent
					ref={contentRef}
					// Focusing the filter would pop the keyboard on touch devices.
					onOpenAutoFocus={(event) => {
						event.preventDefault();
						contentRef.current?.focus();
					}}
					className="h-[min(680px,calc(100dvh-2rem))] gap-0 overflow-hidden p-0 outline-none sm:max-w-5xl"
				>
					<SchemaWorkspace {...props} onDone={() => setOpen(false)} />
				</DialogContent>
			</Dialog>
		</>
	);
}

function SchemaWorkspace({
	onDone,
	...props
}: Readonly<TableSchemaDialogProps & { onDone: () => void }>) {
	const { t } = useTranslation("common");
	const { schema, onGetIndices, onAddColumn } = props;
	const fields = schema?.fields ?? [];
	const { indices, status, reload } = useTableIndices(onGetIndices);
	const [pane, setPane] = useState<SchemaPane>("column");
	const [columnName, setColumnName] = useState<string>();
	// Phones show the rail or the pane, never both.
	const [detailOnPhone, setDetailOnPhone] = useState(false);
	const selected =
		fields.find((field) => field.name === columnName) ?? fields[0];
	const editable = Boolean(
		props.onDropColumns ||
			onAddColumn ||
			props.onAlterColumn ||
			props.onSetPrimaryKey ||
			props.onBuildIndex ||
			props.onDropIndex,
	);

	const openPane = (next: SchemaPane) => {
		setPane(next);
		setDetailOnPhone(true);
	};

	const openColumn = (name?: string) => {
		setColumnName(name);
		openPane("column");
	};

	const handleDropped = (name: string) => {
		const position = fields.findIndex((field) => field.name === name);
		openColumn((fields[position + 1] ?? fields[position - 1])?.name);
		void reload();
	};

	const facts = [
		typeof props.rowCount === "number"
			? t("countRows", {
					count: props.rowCount,
					defaultValue_one: "{{count}} row",
					defaultValue_other: "{{count}} rows",
				})
			: null,
		t("countColumns", {
			count: fields.length,
			defaultValue_one: "{{count}} column",
			defaultValue_other: "{{count}} columns",
		}),
		status === "ready"
			? t("countIndices", {
					count: indices.length,
					defaultValue_one: "{{count}} index",
					defaultValue_other: "{{count}} indices",
				})
			: null,
	]
		.filter(Boolean)
		.join(" · ");

	const renderPane = () => {
		if (pane === "new" && onAddColumn) {
			return (
				<NewColumnPane
					fields={fields}
					rowCount={props.rowCount}
					onAddColumn={onAddColumn}
					onAdded={openColumn}
					onCancel={() => setPane("column")}
				/>
			);
		}
		if (pane === "indices") {
			return (
				<IndicesPane
					fields={fields}
					indices={indices}
					status={status}
					canBuild={Boolean(props.onBuildIndex)}
					onRetry={reload}
					onOpenColumn={openColumn}
					onDropIndex={props.onDropIndex}
				/>
			);
		}
		if (!selected) {
			return (
				<p className="p-6 text-sm text-muted-foreground">
					{t("noSchemaLoadedYet", "No schema loaded yet.")}
				</p>
			);
		}
		return (
			<ColumnPane
				key={selected.name}
				field={selected}
				primaryKey={schema?.primaryKey}
				index={columnIndex(indices, selected.name)}
				indexStatus={status}
				showIndex={Boolean(onGetIndices || props.onBuildIndex)}
				onAlterColumn={props.onAlterColumn}
				onSetPrimaryKey={props.onSetPrimaryKey}
				onBuildIndex={props.onBuildIndex}
				onDropIndex={props.onDropIndex}
				onDropColumns={props.onDropColumns}
				onIndicesChanged={reload}
				onDropped={handleDropped}
			/>
		);
	};

	return (
		<>
			<DialogHeader className="flex-row items-center gap-3 border-b px-5 py-4 pr-12 text-left">
				<div className="flex size-9 shrink-0 items-center justify-center rounded-md border bg-muted/40">
					<Database className="size-4 text-muted-foreground" />
				</div>
				<div className="min-w-0 flex-1 space-y-1">
					<DialogTitle className="truncate text-base">
						{props.tableName ?? schema?.table}
					</DialogTitle>
					<DialogDescription className="truncate text-xs">
						{facts}
					</DialogDescription>
				</div>
			</DialogHeader>
			<div className="flex min-h-0 flex-1 flex-col sm:flex-row">
				<SchemaRail
					className={detailOnPhone ? "hidden sm:flex" : "flex"}
					fields={fields}
					indices={indices}
					primaryKey={schema?.primaryKey}
					selectedName={pane === "column" ? selected?.name : undefined}
					pane={pane}
					canAdd={Boolean(onAddColumn)}
					showIndices={Boolean(onGetIndices)}
					onOpenColumn={openColumn}
					onOpenPane={openPane}
				/>
				<div
					className={cn(
						"min-h-0 min-w-0 flex-1 overflow-y-auto",
						!detailOnPhone && "hidden sm:block",
					)}
				>
					<button
						type="button"
						onClick={() => setDetailOnPhone(false)}
						className="flex items-center gap-1 px-4 pt-3 text-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 sm:hidden"
					>
						<ChevronLeft className="size-4" />
						{t("columns", "Columns")}
					</button>
					{renderPane()}
				</div>
			</div>
			<div className="flex shrink-0 items-center gap-2.5 border-t px-5 py-3">
				{editable ? (
					<History className="size-4 shrink-0 text-muted-foreground" />
				) : (
					<Lock className="size-4 shrink-0 text-muted-foreground" />
				)}
				<p className="flex-1 text-xs text-muted-foreground">
					{editable
						? t(
								"schemaVersionNote",
								"Every change is saved as a new table version. You can roll back from History.",
							)
						: t(
								"schemaReadOnlyNote",
								"Read-only. Schema changes are turned off here.",
							)}
				</p>
				<Button variant="outline" size="sm" onClick={onDone}>
					{t("done", "Done")}
				</Button>
			</div>
		</>
	);
}

function SchemaRail({
	className,
	fields,
	indices,
	primaryKey,
	selectedName,
	pane,
	canAdd,
	showIndices,
	onOpenColumn,
	onOpenPane,
}: Readonly<{
	className?: string;
	fields: LanceField[];
	indices: IIndexConfig[];
	primaryKey?: string;
	selectedName?: string;
	pane: SchemaPane;
	canAdd: boolean;
	showIndices: boolean;
	onOpenColumn: (name: string) => void;
	onOpenPane: (pane: SchemaPane) => void;
}>) {
	const { t } = useTranslation("common");
	const [query, setQuery] = useState("");
	const needle = query.trim().toLowerCase();
	const visible = needle
		? fields.filter((field) => field.name.toLowerCase().includes(needle))
		: fields;

	return (
		<nav
			aria-label={t("columns", "Columns")}
			className={cn(
				"min-h-0 flex-1 flex-col bg-muted/20 sm:w-64 sm:flex-none sm:border-r",
				className,
			)}
		>
			<div className="p-3 pb-2">
				<div className="relative">
					<Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
					<Input
						type="search"
						value={query}
						onChange={(event) => setQuery(event.target.value)}
						placeholder={t("filterColumns", "Filter columns…")}
						aria-label={t("filterColumns", "Filter columns…")}
						className="h-8 pl-8"
					/>
				</div>
			</div>
			{canAdd && (
				<div className="px-2 pb-1">
					<RailButton active={pane === "new"} onClick={() => onOpenPane("new")}>
						<span className="flex size-6 shrink-0 items-center justify-center rounded-md border border-dashed border-muted-foreground/50">
							<Plus className="size-3.5" />
						</span>
						<span className="text-sm font-medium">
							{t("newColumn", "New column")}
						</span>
					</RailButton>
				</div>
			)}
			<div className="flex items-center justify-between px-4 pt-2 pb-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
				<span>{t("columns", "Columns")}</span>
				<span>{visible.length}</span>
			</div>
			<ul className="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-2 pb-2">
				{visible.map((field) => (
					<li key={field.name}>
						<RailButton
							active={field.name === selectedName}
							onClick={() => onOpenColumn(field.name)}
						>
							<span className="min-w-0 flex-1">
								<span className="block truncate font-mono text-[13px]">
									{field.name}
								</span>
								<span className="block truncate text-[11px] text-muted-foreground">
									{fieldTypeLabel(field)} ·{" "}
									{(field.nullable ?? true)
										? t("nullable", "Nullable")
										: t("required", "Required")}
								</span>
							</span>
							{field.name === primaryKey && (
								<KeyRound
									role="img"
									aria-label={t("tableKeyBadge", "Key")}
									className="size-3.5 shrink-0 text-primary"
								/>
							)}
							{columnIndex(indices, field.name) && (
								<Zap
									role="img"
									aria-label={t("schemaIndexed", "Indexed")}
									className="size-3.5 shrink-0 text-primary"
								/>
							)}
						</RailButton>
					</li>
				))}
				{visible.length === 0 && (
					<li className="px-2 py-6 text-center text-xs text-muted-foreground">
						{t("schemaNoColumnsMatch", "No columns match")}
					</li>
				)}
			</ul>
			{showIndices && (
				<div className="border-t p-2">
					<RailButton
						active={pane === "indices"}
						onClick={() => onOpenPane("indices")}
					>
						<Zap className="size-4 shrink-0 text-primary" />
						<span className="flex-1 text-sm font-medium">
							{t("indices", "Indices")}
						</span>
						<span className="text-xs text-muted-foreground">
							{indices.length}
						</span>
					</RailButton>
				</div>
			)}
		</nav>
	);
}

function RailButton({
	active,
	className,
	...props
}: React.ComponentProps<"button"> & { active: boolean }) {
	return (
		<button
			type="button"
			aria-current={active || undefined}
			className={cn(
				"flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left outline-none transition-colors hover:bg-muted/60 focus-visible:ring-[3px] focus-visible:ring-ring/50",
				active && "bg-muted",
				className,
			)}
			{...props}
		/>
	);
}

function ColumnPane({
	field,
	primaryKey,
	index,
	indexStatus,
	showIndex,
	onAlterColumn,
	onSetPrimaryKey,
	onBuildIndex,
	onDropIndex,
	onDropColumns,
	onIndicesChanged,
	onDropped,
}: Readonly<{
	field: LanceField;
	primaryKey?: string;
	index?: IIndexConfig;
	indexStatus: IndexStatus;
	showIndex: boolean;
	onAlterColumn?: (column: string, nullable: boolean) => Promise<void>;
	onSetPrimaryKey?: (column: string) => Promise<void>;
	onBuildIndex?: (column: string, indexType: string) => Promise<void>;
	onDropIndex?: (indexName: string) => Promise<void>;
	onDropColumns?: (columns: string[]) => Promise<void>;
	onIndicesChanged: () => Promise<void>;
	onDropped: (name: string) => void;
}>) {
	const { t } = useTranslation("common");
	const titleId = useId();
	const [confirm, setConfirm] = useState<ColumnConfirm | null>(null);
	const [busy, setBusy] = useState<ColumnAction | null>(null);
	const [indexType, setIndexType] = useState("auto");
	const nullable = field.nullable ?? true;
	const isKey = field.name === primaryKey;
	const canBecomeKey = !primaryKey && Boolean(field.keyEligible);
	const indexKind = field.indexKind ?? field.kind;
	const unsupportedIndex = indexKind === "unsupported-geometry";

	const run = async (action: ColumnAction, task: () => Promise<void>) => {
		setBusy(action);
		const ok = await attempt(task);
		setBusy(null);
		if (ok) setConfirm(null);
		return ok;
	};

	const allowNull = () =>
		onAlterColumn && run("nullable", () => onAlterColumn(field.name, true));
	// Lance refuses while any row holds NULL; the host reports that error.
	const makeRequired = () =>
		onAlterColumn && run("required", () => onAlterColumn(field.name, false));
	const setKey = () =>
		onSetPrimaryKey && run("key", () => onSetPrimaryKey(field.name));
	const buildIndex = async () => {
		if (!onBuildIndex) return;
		if (await run("buildIndex", () => onBuildIndex(field.name, indexType)))
			await onIndicesChanged();
	};
	const dropIndex = async () => {
		if (!onDropIndex || !index) return;
		if (await run("dropIndex", () => onDropIndex(index.name)))
			await onIndicesChanged();
	};
	const dropColumn = async () => {
		if (!onDropColumns) return;
		if (await run("drop", () => onDropColumns([field.name])))
			onDropped(field.name);
	};

	const nullableAction = (() => {
		if (nullable) {
			if (!onAlterColumn) return null;
			return (
				<Button
					variant="outline"
					size="sm"
					disabled={busy !== null}
					onClick={() => void makeRequired()}
				>
					{busy === "required" && <Loader2 className="animate-spin" />}
					{t("schemaMakeRequired", "Make required")}
				</Button>
			);
		}
		if (isKey)
			return (
				<LockNote>{t("schemaKeyStaysRequired", "Key stays required")}</LockNote>
			);
		if (!onAlterColumn || confirm === "nullable") return null;
		return (
			<Button
				variant="outline"
				size="sm"
				disabled={busy !== null}
				onClick={() => setConfirm("nullable")}
			>
				{t("schemaAllowNull", "Allow NULL")}
			</Button>
		);
	})();

	const keyText = (() => {
		if (isKey)
			return t(
				"tableKeyCurrent",
				'"{{column}}" is the key. A key can\'t be changed or removed.',
				{ column: field.name },
			);
		if (canBecomeKey)
			return t(
				"tableKeyDescription",
				"Concurrent Upserts on the key column can't create duplicate rows. An Upsert makes its ID column the key when the table has none.",
			);
		return t(
			"tableKeyIneligible",
			"Only required text, 32/64-bit integer or binary columns can be the key.",
		);
	})();

	const keyAction = (() => {
		if (isKey) return <LockNote>{t("schemaPermanent", "Permanent")}</LockNote>;
		if (!onSetPrimaryKey || confirm === "key") return null;
		return (
			<Button
				variant="outline"
				size="sm"
				disabled={!canBecomeKey || busy !== null}
				onClick={() => setConfirm("key")}
			>
				<KeyRound /> {t("tableKeySetAction", "Set as key")}
			</Button>
		);
	})();

	const indexValue = (() => {
		if (index) {
			return (
				<div className="flex min-w-0 flex-col gap-1">
					<span className="flex min-w-0 items-center gap-2">
						<IndexChip>{indexLabel(index)}</IndexChip>
						<span className="truncate font-mono text-xs text-muted-foreground">
							{index.name}
						</span>
					</span>
					{indexOption(index)?.description && (
						<span className="text-xs text-muted-foreground">
							{indexOption(index)?.description}
						</span>
					)}
				</div>
			);
		}
		if (indexStatus === "loading" || indexStatus === "idle") {
			return (
				<span className="inline-flex items-center gap-2 text-xs text-muted-foreground">
					<Loader2 className="size-3.5 animate-spin" />
					{t("loading", "Loading…")}
				</span>
			);
		}
		if (indexStatus === "error") {
			return (
				<span className="text-xs text-muted-foreground">
					{t("schemaIndicesFailed", "Couldn't load the indices.")}
				</span>
			);
		}
		if (!onBuildIndex) {
			return (
				<span className="text-xs text-muted-foreground">
					{t("none", "None")}
				</span>
			);
		}
		return (
			<div className="flex min-w-0 flex-col gap-1.5">
				<IndexTypeSelect
					value={indexType}
					onChange={setIndexType}
					className="w-56 max-w-full"
					columnType={indexKind}
					disabled={busy !== null || unsupportedIndex}
				/>
				<IndexTypeHelp value={indexType} columnType={indexKind} />
			</div>
		);
	})();

	const indexAction = (() => {
		if (index && onDropIndex) {
			return (
				<Button
					variant="ghost"
					size="sm"
					className="text-destructive hover:text-destructive"
					disabled={busy !== null}
					onClick={() => void dropIndex()}
				>
					{busy === "dropIndex" && <Loader2 className="animate-spin" />}
					{t("dropIndex", "Drop index")}
				</Button>
			);
		}
		if (index || !onBuildIndex || indexStatus !== "ready") return null;
		return (
			<Button
				variant="outline"
				size="sm"
				disabled={busy !== null || unsupportedIndex}
				onClick={() => void buildIndex()}
			>
				{busy === "buildIndex" ? <Loader2 className="animate-spin" /> : <Zap />}
				{t("buildIndex", "Build index")}
			</Button>
		);
	})();

	return (
		<section aria-labelledby={titleId} className="flex flex-col gap-5 p-6">
			<h3
				id={titleId}
				className="flex min-w-0 items-center gap-2.5 font-mono text-xl font-medium"
			>
				<span className="truncate">{field.name}</span>
				{isKey && <KeyBadge />}
			</h3>

			<div className="divide-y rounded-lg border bg-card/40">
				<PropertyRow
					label={t("schemaTypeLabel", "Type")}
					hint={t("schemaTypeHint", "Set when the column is created")}
					value={<TypeChip>{fieldTypeLabel(field)}</TypeChip>}
					action={<LockNote>{t("schemaTypeFixed", "Can't change")}</LockNote>}
				/>
				<PropertyRow
					label={t("schemaEmptyValues", "Empty values")}
					hint={t("schemaEmptyValuesHint", "Whether rows can hold NULL")}
					value={<NullabilityValue nullable={nullable} />}
					action={nullableAction}
				>
					{confirm === "nullable" && (
						<ConfirmPanel
							title={t("schemaAllowNullTitle", "Allow NULL in {{name}}?", {
								name: field.name,
							})}
							description={[
								t(
									"schemaAllowNullBody",
									"Rows can then leave it empty. Making it required again only works while no row holds NULL.",
								),
								canBecomeKey && onSetPrimaryKey
									? t(
											"schemaAllowNullLosesKey",
											"While it's nullable, it can't be the table key.",
										)
									: null,
							]
								.filter(Boolean)
								.join(" ")}
							confirmLabel={t("schemaAllowNull", "Allow NULL")}
							busy={busy === "nullable"}
							onCancel={() => setConfirm(null)}
							onConfirm={() => void allowNull()}
						/>
					)}
				</PropertyRow>
				{(isKey || (!primaryKey && onSetPrimaryKey)) && (
					<PropertyRow
						label={t("schemaTableKey", "Table key")}
						hint={t("schemaTableKeyHint", "Stops duplicate rows from Upserts")}
						value={
							<span className="text-xs text-muted-foreground">{keyText}</span>
						}
						action={keyAction}
					>
						{confirm === "key" && (
							<ConfirmPanel
								title={t(
									"tableKeyConfirmTitle",
									'Make "{{column}}" the table key?',
									{ column: field.name },
								)}
								description={`${t(
									"tableKeyConfirmDescription",
									"The key can't be changed or removed afterwards. Concurrent Upserts on this column stop creating duplicate rows; Upserts on other columns are not protected.",
								)} ${t(
									"tableKeyConfirmDuplicates",
									"This fails if the column already holds duplicate values. Remove them first.",
								)}`}
								confirmLabel={t("tableKeySetAction", "Set as key")}
								busy={busy === "key"}
								onCancel={() => setConfirm(null)}
								onConfirm={() => void setKey()}
							/>
						)}
					</PropertyRow>
				)}
				{showIndex && (
					<PropertyRow
						label={t("schemaIndexLabel", "Index")}
						hint={t("schemaIndexHint", "Speeds up filters and search")}
						value={indexValue}
						action={indexAction}
					/>
				)}
			</div>

			{onDropColumns && isKey && (
				<div className="flex flex-col gap-3 rounded-lg border p-4 sm:flex-row sm:items-center">
					<div className="min-w-0 flex-1 space-y-1">
						<p className="text-sm font-medium">
							{t("schemaDropColumn", "Drop column")}
						</p>
						<p className="text-xs leading-relaxed text-muted-foreground">
							{t(
								"schemaKeyCannotBeDropped",
								"The table key can't be dropped. Upserts rely on it to keep rows unique.",
							)}
						</p>
					</div>
					<LockNote>{t("schemaPermanent", "Permanent")}</LockNote>
				</div>
			)}
			{onDropColumns &&
				!isKey &&
				(confirm === "drop" ? (
					<ConfirmPanel
						destructive
						className="m-0"
						title={t("schemaDropColumnTitle", "Drop {{name}}?", {
							name: field.name,
						})}
						description={dropDescription(t, index)}
						confirmLabel={t("schemaDropColumnNamed", "Drop {{name}}", {
							name: field.name,
						})}
						busy={busy === "drop"}
						onCancel={() => setConfirm(null)}
						onConfirm={() => void dropColumn()}
					/>
				) : (
					<div className="flex flex-col gap-3 rounded-lg border border-destructive/30 p-4 sm:flex-row sm:items-center">
						<div className="min-w-0 flex-1 space-y-1">
							<p className="text-sm font-medium">
								{t("schemaDropColumn", "Drop column")}
							</p>
							<p className="text-xs leading-relaxed text-muted-foreground">
								{dropDescription(t, index)}
							</p>
						</div>
						<Button
							variant="outline"
							size="sm"
							className="shrink-0 text-destructive hover:text-destructive"
							disabled={busy !== null}
							onClick={() => setConfirm("drop")}
						>
							<Trash2 /> {t("schemaDropColumn", "Drop column")}
						</Button>
					</div>
				))}
		</section>
	);
}

function dropDescription(
	t: ReturnType<typeof useTranslation>["t"],
	index?: IIndexConfig,
) {
	return index
		? t(
				"schemaDropColumnBodyIndexed",
				"Deletes the column and its {{index}} index. Flows that read it will fail. To undo, restore the previous version from History.",
				{ index: indexLabel(index) },
			)
		: t(
				"schemaDropColumnBody",
				"Deletes the column and its values. Flows that read it will fail. To undo, restore the previous version from History.",
			);
}

function PropertyRow({
	label,
	hint,
	value,
	action,
	children,
}: Readonly<{
	label: string;
	hint: string;
	value: React.ReactNode;
	action?: React.ReactNode;
	children?: React.ReactNode;
}>) {
	return (
		<div>
			<div className="grid gap-x-5 gap-y-2 px-4 py-3.5 sm:grid-cols-[11rem_minmax(0,1fr)_auto] sm:items-center">
				<div className="space-y-0.5">
					<p className="text-sm font-medium">{label}</p>
					<p className="text-xs text-muted-foreground">{hint}</p>
				</div>
				<div className="min-w-0">{value}</div>
				<div className="flex sm:justify-end">{action}</div>
			</div>
			{children}
		</div>
	);
}

function ConfirmPanel({
	title,
	description,
	confirmLabel,
	busy,
	destructive = false,
	className,
	onCancel,
	onConfirm,
}: Readonly<{
	title: string;
	description: string;
	confirmLabel: string;
	busy: boolean;
	destructive?: boolean;
	className?: string;
	onCancel: () => void;
	onConfirm: () => void;
}>) {
	const { t } = useTranslation("common");
	const cancelRef = useRef<HTMLButtonElement>(null);
	useEffect(() => {
		cancelRef.current?.focus();
	}, []);

	return (
		<div
			className={cn(
				"mx-4 mb-4 flex flex-col gap-3 rounded-md border p-3.5 sm:flex-row sm:items-start",
				destructive
					? "border-destructive/40 bg-destructive/5"
					: "border-primary/30 bg-primary/5",
				className,
			)}
		>
			<AlertTriangle
				className={cn(
					"mt-0.5 size-4 shrink-0",
					destructive ? "text-destructive" : "text-primary",
				)}
			/>
			<div className="min-w-0 flex-1 space-y-1">
				<p className="text-sm font-medium">{title}</p>
				<p className="text-xs leading-relaxed text-muted-foreground">
					{description}
				</p>
			</div>
			<div className="flex shrink-0 gap-2">
				<Button
					ref={cancelRef}
					variant="ghost"
					size="sm"
					disabled={busy}
					onClick={onCancel}
				>
					{t("cancel", "Cancel")}
				</Button>
				<Button
					variant={destructive ? "destructive" : "default"}
					size="sm"
					disabled={busy}
					onClick={onConfirm}
				>
					{busy && <Loader2 className="animate-spin" />}
					{confirmLabel}
				</Button>
			</div>
		</div>
	);
}

function NullabilityValue({ nullable }: Readonly<{ nullable: boolean }>) {
	const { t } = useTranslation("common");
	return (
		<span className="flex flex-wrap items-center gap-2.5">
			<span
				className={cn(
					"inline-flex h-6 items-center rounded-full border px-2.5 text-xs font-medium",
					nullable
						? "border-primary/30 bg-primary/10 text-primary"
						: "bg-muted text-foreground",
				)}
			>
				{nullable ? t("nullable", "Nullable") : t("required", "Required")}
			</span>
			<span className="text-xs text-muted-foreground">
				{nullable
					? t("schemaNullableHint", "Rows may leave it empty")
					: t("schemaRequiredHint", "Every row has a value")}
			</span>
		</span>
	);
}

function KeyBadge() {
	const { t } = useTranslation("common");
	return (
		<Badge variant="secondary" className="shrink-0 gap-1 font-sans">
			<KeyRound className="size-3" />
			{t("tableKeyBadge", "Key")}
		</Badge>
	);
}

function LockNote({ children }: Readonly<{ children: React.ReactNode }>) {
	return (
		<span className="inline-flex items-center gap-1.5 whitespace-nowrap text-xs text-muted-foreground">
			<Lock className="size-3.5" />
			{children}
		</span>
	);
}

function TypeChip({ children }: Readonly<{ children: React.ReactNode }>) {
	return (
		<Badge variant="outline" className="font-mono text-[11px] font-normal">
			{children}
		</Badge>
	);
}

function IndexChip({ children }: Readonly<{ children: React.ReactNode }>) {
	return (
		<span className="inline-flex shrink-0 items-center gap-1 rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
			<Zap className="size-3" />
			{children}
		</span>
	);
}

const ADD_COLUMN_TYPES = EDIT_COLUMN_TYPE_GROUPS.flatMap(
	(group) => group.options,
);

const DEFAULT_PLACEHOLDERS: Record<string, string> = {
	string: "unknown",
	int32: "0",
	int64: "0",
	float32: "0.5",
	float64: "0.5",
	date32: "2026-01-31",
	timestamp: "2026-01-31 14:30:00",
};

function NewColumnPane({
	fields,
	rowCount,
	onAddColumn,
	onAdded,
	onCancel,
}: Readonly<{
	fields: LanceField[];
	rowCount?: number;
	onAddColumn: (name: string, sqlExpression: string) => Promise<void>;
	onAdded: (name: string) => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("common");
	const ids = useId();
	const nameRef = useRef<HTMLInputElement>(null);
	const [name, setName] = useState("");
	const [type, setType] = useState("string");
	const [fill, setFill] = useState("null");
	const [value, setValue] = useState("");
	const [submitted, setSubmitted] = useState(false);
	const [busy, setBusy] = useState(false);

	useEffect(() => {
		nameRef.current?.focus();
	}, []);

	const trimmedName = name.trim();
	const isBoolean = type === "boolean";
	const fillValue =
		fill === "value" ? (isBoolean ? value || "true" : value) : "";
	const nameError =
		validateColumnName(name) ??
		(fields.some((field) => field.name === trimmedName)
			? t("schemaColumnExists", 'A column named "{{name}}" already exists', {
					name: trimmedName,
				})
			: null);
	const valueError =
		fill === "value" && !fillValue.trim()
			? t("schemaEnterAValue", "Enter a value")
			: validateColumnDefault(type, fillValue);
	const expression = buildAddColumnExpression(type, fillValue);
	const showNameError = Boolean(nameError) && (submitted || name.length > 0);
	const showValueError =
		Boolean(valueError) && (submitted || value.trim().length > 0);
	const fillLegend = rowCount
		? t("schemaValueForExistingRows", {
				count: rowCount,
				defaultValue_one: "Value for the {{count}} existing row",
				defaultValue_other: "Value for the {{count}} existing rows",
			})
		: t("schemaValueForExistingRowsUnknown", "Value for existing rows");
	const defaultLabel = t("schemaDefaultValue", "Default value");

	const submit = async (event: React.FormEvent) => {
		event.preventDefault();
		setSubmitted(true);
		if (nameError || valueError || !expression) return;
		setBusy(true);
		const ok = await attempt(() => onAddColumn(trimmedName, expression));
		setBusy(false);
		if (ok) onAdded(trimmedName);
	};

	return (
		<form
			noValidate
			onSubmit={(event) => void submit(event)}
			className="flex min-h-full flex-col"
		>
			<div className="flex flex-1 flex-col gap-6 p-6">
				<div className="space-y-1">
					<h3 className="text-lg font-semibold">
						{t("newColumn", "New column")}
					</h3>
					<p className="text-sm text-muted-foreground">
						{t(
							"schemaNewColumnDescription",
							"Added as a nullable column. Existing rows get the value you choose below.",
						)}
					</p>
				</div>

				<div className="space-y-2">
					<Label htmlFor={`${ids}-name`}>{t("schemaColumnName", "Name")}</Label>
					<Input
						ref={nameRef}
						id={`${ids}-name`}
						className="max-w-sm font-mono"
						placeholder="column_name"
						autoComplete="off"
						spellCheck={false}
						value={name}
						onChange={(event) => setName(event.target.value)}
						aria-invalid={showNameError}
						aria-describedby={`${ids}-name-hint`}
					/>
					<p
						id={`${ids}-name-hint`}
						className={cn(
							"text-xs",
							showNameError ? "text-destructive" : "text-muted-foreground",
						)}
					>
						{showNameError
							? nameError
							: t(
									"schemaColumnNameHint",
									"Letters, numbers and underscores. It can't start with a number.",
								)}
					</p>
				</div>

				<fieldset className="space-y-2">
					<legend className="mb-2 text-sm font-medium">
						{t("schemaTypeLabel", "Type")}
					</legend>
					<RadioGroupPrimitive.Root
						aria-label={t("schemaTypeLabel", "Type")}
						value={type}
						onValueChange={(next) => {
							setType(next);
							setValue("");
						}}
						className="grid grid-cols-2 gap-2 sm:grid-cols-3"
					>
						{ADD_COLUMN_TYPES.map((option) => (
							<RadioGroupPrimitive.Item
								key={option.value}
								value={option.value}
								className="flex h-9 items-center justify-between gap-2 rounded-md border bg-background px-3 text-left outline-none transition-colors hover:border-foreground/30 focus-visible:ring-[3px] focus-visible:ring-ring/50 data-[state=checked]:border-primary data-[state=checked]:bg-primary/10"
							>
								<span className="text-sm font-medium">{option.label}</span>
								<span className="font-mono text-[11px] text-muted-foreground">
									{addColumnSqlType(option.value)}
								</span>
							</RadioGroupPrimitive.Item>
						))}
					</RadioGroupPrimitive.Root>
					<p className="text-xs text-muted-foreground">
						{t(
							"schemaVectorGeometryNote",
							"Vector and geometry columns can only be defined when the table is created.",
						)}
					</p>
				</fieldset>

				<fieldset className="space-y-2">
					<legend className="mb-2 text-sm font-medium">{fillLegend}</legend>
					<div className="flex flex-wrap items-center gap-3">
						<Segmented
							label={fillLegend}
							value={fill}
							onValueChange={setFill}
							options={[
								{ value: "null", label: "NULL" },
								{ value: "value", label: t("schemaFillWithValue", "A value") },
							]}
						/>
						{fill === "value" && isBoolean && (
							<Segmented
								label={defaultLabel}
								value={fillValue}
								onValueChange={setValue}
								options={[
									{ value: "true", label: "true" },
									{ value: "false", label: "false" },
								]}
							/>
						)}
						{fill === "value" && !isBoolean && (
							<Input
								className="max-w-xs font-mono"
								aria-label={defaultLabel}
								aria-invalid={showValueError}
								placeholder={DEFAULT_PLACEHOLDERS[type]}
								value={value}
								onChange={(event) => setValue(event.target.value)}
							/>
						)}
					</div>
					{showValueError && (
						<p className="text-xs text-destructive">{valueError}</p>
					)}
				</fieldset>
			</div>

			<div className="sticky bottom-0 flex flex-wrap items-center gap-3 border-t bg-background px-6 py-3">
				<span className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
					SQL
				</span>
				<code className="min-w-0 flex-1 break-all font-mono text-xs">
					{expression}
				</code>
				<div className="flex gap-2">
					<Button
						type="button"
						variant="ghost"
						disabled={busy}
						onClick={onCancel}
					>
						{t("cancel", "Cancel")}
					</Button>
					<Button type="submit" disabled={busy}>
						{busy && <Loader2 className="animate-spin" />}
						{t("addColumn", "Add Column")}
					</Button>
				</div>
			</div>
		</form>
	);
}

function Segmented({
	label,
	value,
	options,
	onValueChange,
}: Readonly<{
	label: string;
	value: string;
	options: readonly { value: string; label: string }[];
	onValueChange: (value: string) => void;
}>) {
	return (
		<RadioGroupPrimitive.Root
			aria-label={label}
			orientation="horizontal"
			value={value}
			onValueChange={onValueChange}
			className="inline-flex rounded-md border bg-muted/40 p-0.5"
		>
			{options.map((option) => (
				<RadioGroupPrimitive.Item
					key={option.value}
					value={option.value}
					className="h-7 rounded-[5px] px-3 text-xs font-medium text-muted-foreground outline-none transition-colors focus-visible:ring-[3px] focus-visible:ring-ring/50 data-[state=checked]:bg-background data-[state=checked]:text-foreground"
				>
					{option.label}
				</RadioGroupPrimitive.Item>
			))}
		</RadioGroupPrimitive.Root>
	);
}

function IndicesPane({
	fields,
	indices,
	status,
	canBuild,
	onRetry,
	onOpenColumn,
	onDropIndex,
}: Readonly<{
	fields: LanceField[];
	indices: IIndexConfig[];
	status: IndexStatus;
	canBuild: boolean;
	onRetry: () => Promise<void>;
	onOpenColumn: (name: string) => void;
	onDropIndex?: (indexName: string) => Promise<void>;
}>) {
	const { t } = useTranslation("common");
	const titleId = useId();
	const [dropping, setDropping] = useState<string | null>(null);
	const nested = indices.filter(
		(index) =>
			!index.columns.some((column) =>
				fields.some((field) => field.name === column),
			),
	);

	const drop = async (name: string) => {
		if (!onDropIndex) return;
		setDropping(name);
		if (await attempt(() => onDropIndex(name))) await onRetry();
		setDropping(null);
	};

	const indexCell = (index?: IIndexConfig) => {
		if (index) {
			return (
				<span className="flex min-w-0 items-center gap-2">
					<IndexChip>{indexLabel(index)}</IndexChip>
					<span className="truncate font-mono text-xs text-muted-foreground">
						{index.name}
					</span>
				</span>
			);
		}
		return (
			<span className="text-xs text-muted-foreground">
				{status === "ready" ? t("none", "None") : t("loading", "Loading…")}
			</span>
		);
	};

	const actionCell = (field: string, index?: IIndexConfig) => {
		if (index && onDropIndex) {
			return (
				<Button
					variant="ghost"
					size="sm"
					className="text-destructive hover:text-destructive"
					disabled={dropping !== null}
					aria-label={t("schemaDropIndexNamed", "Drop index {{name}}", {
						name: index.name,
					})}
					onClick={() => void drop(index.name)}
				>
					{dropping === index.name && <Loader2 className="animate-spin" />}
					{t("dropIndex", "Drop index")}
				</Button>
			);
		}
		if (index || !canBuild || status !== "ready") return null;
		return (
			<Button variant="ghost" size="sm" onClick={() => onOpenColumn(field)}>
				{t("schemaAddIndex", "Add index")}
			</Button>
		);
	};

	return (
		<section aria-labelledby={titleId} className="flex flex-col gap-5 p-6">
			<div className="space-y-1">
				<h3 id={titleId} className="text-lg font-semibold">
					{t("indices", "Indices")}
				</h3>
				<p className="text-sm text-muted-foreground">
					{t(
						"schemaIndicesDescription",
						"Indices speed up filters, text search and similarity queries. Building one reads every row.",
					)}
				</p>
			</div>
			{status === "error" ? (
				<div className="flex items-center gap-3 rounded-lg border border-destructive/40 p-4 text-sm">
					<AlertTriangle className="size-4 shrink-0 text-destructive" />
					<span className="flex-1">
						{t("schemaIndicesFailed", "Couldn't load the indices.")}
					</span>
					<Button variant="outline" size="sm" onClick={() => void onRetry()}>
						{t("retry", "Retry")}
					</Button>
				</div>
			) : (
				<div className="rounded-lg border">
					{/* Resets the base-layer table rules in global.css (cell borders, margins). */}
					<Table className="my-0">
						<TableHeader className="bg-muted/30">
							<TableRow className="hover:bg-transparent">
								<TableHead className="border-0 bg-transparentpx-4">
									{t("column", "Column")}
								</TableHead>
								<TableHead className="border-0 bg-transparentpx-4">
									{t("schemaTypeLabel", "Type")}
								</TableHead>
								<TableHead className="border-0 bg-transparentpx-4">
									{t("schemaIndexLabel", "Index")}
								</TableHead>
								<TableHead className="border-0 bg-transparentpx-4">
									<span className="sr-only">{t("actions", "Actions")}</span>
								</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{fields.map((field) => {
								const index = columnIndex(indices, field.name);
								return (
									<TableRow key={field.name}>
										<TableCell className="border-0 bg-transparentpx-4">
											<button
												type="button"
												className="rounded-sm font-mono text-[13px] underline decoration-muted-foreground/40 underline-offset-4 outline-none hover:decoration-primary focus-visible:ring-[3px] focus-visible:ring-ring/50"
												onClick={() => onOpenColumn(field.name)}
											>
												{field.name}
											</button>
										</TableCell>
										<TableCell className="border-0 bg-transparentpx-4">
											<TypeChip>{fieldTypeLabel(field)}</TypeChip>
										</TableCell>
										<TableCell className="border-0 bg-transparentmax-w-64 px-4">
											{indexCell(index)}
										</TableCell>
										<TableCell className="border-0 bg-transparentpx-4 text-right">
											{actionCell(field.name, index)}
										</TableCell>
									</TableRow>
								);
							})}
							{nested.map((index) => (
								<TableRow key={index.name}>
									<TableCell className="border-0 bg-transparentpx-4 font-mono text-[13px]">
										{index.columns.join(", ")}
									</TableCell>
									<TableCell className="border-0 bg-transparentpx-4" />
									<TableCell className="border-0 bg-transparentmax-w-64 px-4">
										{indexCell(index)}
									</TableCell>
									<TableCell className="border-0 bg-transparentpx-4 text-right">
										{actionCell(index.columns[0] ?? "", index)}
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				</div>
			)}
		</section>
	);
}
