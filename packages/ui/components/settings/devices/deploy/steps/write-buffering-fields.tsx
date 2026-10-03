"use client";

import { useTranslation } from "@flow-like/locales";
import { Info, Plus, X } from "lucide-react";
import { useId } from "react";
import type { OfflineWritesConfig } from "../../../../../lib/device-management/deployment";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../../ui/select";
import type { DevicesT } from "../../primitives/area-context";
import { DvButton } from "../../primitives/dv-button";
import {
	DvInput,
	Field,
	InputWithUnit,
	SwitchField,
} from "../../primitives/form-fields";

/* Write buffering (APP §3.10.4): which tables and folders keep accepting changes while the internet is down. */

type Table = OfflineWritesConfig["tables"][number];
type Folder = OfflineWritesConfig["files"][number];

const MIB = 1024 ** 2;
const GIB = 1024 ** 3;
const DAY = 86_400;

export const DEFAULT_WRITE_BUFFERING: OfflineWritesConfig = {
	tables: [],
	files: [],
	max_queue_bytes: 256 * MIB,
	max_operations: 10_000,
	max_age_seconds: 7 * DAY,
	max_mirror_bytes: 2 * GIB,
};

const TABLE_NAME = /^[A-Za-z0-9_-]{1,128}$/u;
const KEY_COLUMN = /^[A-Za-z0-9_]{1,128}$/u;

function plainSegment(part: string): boolean {
	return ![".", "..", ""].includes(part) && !part.endsWith(".lance");
}

function folderValid(folder: Folder): boolean {
	const parts = folder.prefix.split("/");
	const database =
		["storage", "user"].includes(folder.purpose) && parts[0] === "db";
	const escaped = /[\\%\0]/u.test(folder.prefix);
	return parts.every(plainSegment) && !database && !escaped;
}

function tableKey(table: Table): string {
	return [table.purpose, table.table].join("/");
}

function badTableName(row: Table): boolean {
	return !TABLE_NAME.test(row.table);
}

function badKeyColumn(row: Table): boolean {
	return !KEY_COLUMN.test(row.primary_key);
}

function badFolder(folder: Folder): boolean {
	return !folderValid(folder);
}

export type WriteBufferingIssue =
	| "nothing_selected"
	| "table_name"
	| "key_column"
	| "table_twice"
	| "folder";

/** What keeps the selection from being sent; the plan's own check blocks Continue with the same rules. */
export function writeBufferingIssues(
	value: OfflineWritesConfig,
): WriteBufferingIssue[] {
	const names = value.tables.map(tableKey);
	const issues: WriteBufferingIssue[] = [];
	if (value.tables.length + value.files.length === 0)
		issues.push("nothing_selected");
	if (value.tables.some(badTableName)) issues.push("table_name");
	if (value.tables.some(badKeyColumn)) issues.push("key_column");
	if (new Set(names).size !== names.length) issues.push("table_twice");
	if (value.files.some(badFolder)) issues.push("folder");
	return issues;
}

const ISSUE_COPY: Record<WriteBufferingIssue, (t: DevicesT) => string> = {
	nothing_selected: (t) =>
		t(
			"devices:deployShip.writes.issueNothing",
			"Add at least one table or folder, or turn write buffering off.",
		),
	table_name: (t) =>
		t(
			"devices:deployShip.writes.issueTable",
			"Table names use letters, digits, - and _.",
		),
	key_column: (t) =>
		t(
			"devices:deployShip.writes.issueKey",
			"Each table needs its key column (letters, digits and _).",
		),
	table_twice: (t) =>
		t("devices:deployShip.writes.issueTwice", "Each table can be listed once."),
	folder: (t) =>
		t(
			"devices:deployShip.writes.issueFolder",
			"Folders are paths inside the area, like exports/daily, and never a table's folder.",
		),
};

interface AreaOption<T extends string> {
	value: T;
	label: string;
}

interface AreaSelectProps<T extends string> {
	label: string;
	value: T;
	options: readonly AreaOption<T>[];
	disabled: boolean;
	onChange(value: T): void;
}

function AreaSelect<T extends string>({
	label,
	value,
	options,
	disabled,
	onChange,
}: AreaSelectProps<T>) {
	return (
		<Select
			value={value}
			disabled={disabled}
			onValueChange={(next) => onChange(next as T)}
		>
			<SelectTrigger
				aria-label={label}
				className="h-8.5 w-full min-w-0 rounded-lg border-input bg-card px-2.5 text-ui shadow-none"
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-none">
				{options.map((option) => (
					<SelectItem
						key={option.value}
						value={option.value}
						className="focus:bg-row-hover focus:text-foreground"
					>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

interface ListProps<T> {
	rows: readonly T[];
	disabled: boolean;
	onChange(rows: T[]): void;
}

function replaceAt<T>(rows: readonly T[], index: number, row: T): T[] {
	return rows.map((value, position) => (position === index ? row : value));
}

function removeAt<T>(rows: readonly T[], index: number): T[] {
	return rows.filter((_, position) => position !== index);
}

function TableRows({ rows, disabled, onChange }: Readonly<ListProps<Table>>) {
	const { t } = useTranslation("devices");
	const areas: AreaOption<Table["purpose"]>[] = [
		{
			value: "storage",
			label: t("deployShip.writes.areaStorage", "Project storage"),
		},
		{ value: "user", label: t("deployShip.writes.areaUser", "User data") },
	];
	return (
		<div className="flex flex-col gap-1.5">
			{rows.map((row, index) => (
				<div
					// biome-ignore lint/suspicious/noArrayIndexKey: rows have no identity besides their position while they are typed
					key={index}
					className="grid grid-cols-[150px_minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-1.5 @max-[520px]/devices:grid-cols-1"
				>
					<AreaSelect
						label={t("deployShip.writes.tableArea", "Table {{n}} area", {
							n: index + 1,
						})}
						value={row.purpose}
						options={areas}
						disabled={disabled}
						onChange={(purpose) =>
							onChange(replaceAt(rows, index, { ...row, purpose }))
						}
					/>
					<DvInput
						mono
						disabled={disabled}
						placeholder={t("deployShip.writes.tablePlaceholder", "table")}
						aria-label={t("deployShip.writes.tableName", "Table {{n}} name", {
							n: index + 1,
						})}
						aria-invalid={!TABLE_NAME.test(row.table)}
						value={row.table}
						onChange={(event) =>
							onChange(
								replaceAt(rows, index, { ...row, table: event.target.value }),
							)
						}
					/>
					<DvInput
						mono
						disabled={disabled}
						placeholder={t("deployShip.writes.keyPlaceholder", "key column")}
						aria-label={t(
							"deployShip.writes.tableKey",
							"Table {{n}} key column",
							{
								n: index + 1,
							},
						)}
						aria-invalid={!KEY_COLUMN.test(row.primary_key)}
						value={row.primary_key}
						onChange={(event) =>
							onChange(
								replaceAt(rows, index, {
									...row,
									primary_key: event.target.value,
								}),
							)
						}
					/>
					<DvButton
						size="xs"
						variant="ghost"
						iconOnly
						icon={X}
						disabled={disabled}
						aria-label={t(
							"deployShip.writes.removeTable",
							"Remove table {{n}}",
							{
								n: index + 1,
							},
						)}
						onClick={() => onChange(removeAt(rows, index))}
					/>
				</div>
			))}
			<DvButton
				size="sm"
				variant="ghost"
				icon={Plus}
				className="self-start"
				disabled={disabled}
				onClick={() =>
					onChange([
						...rows,
						{ purpose: "storage", database: "db", table: "", primary_key: "" },
					])
				}
			>
				{t("deployShip.writes.addTable", "Add table")}
			</DvButton>
		</div>
	);
}

function FolderRows({ rows, disabled, onChange }: Readonly<ListProps<Folder>>) {
	const { t } = useTranslation("devices");
	const areas: AreaOption<Folder["purpose"]>[] = [
		{
			value: "files",
			label: t("deployShip.writes.areaFiles", "Project files"),
		},
		{
			value: "storage",
			label: t("deployShip.writes.areaStorage", "Project storage"),
		},
		{
			value: "user",
			label: t("deployShip.writes.areaUserFiles", "User files"),
		},
		{
			value: "temporary",
			label: t("deployShip.writes.areaTemporary", "Temporary files"),
		},
	];
	return (
		<div className="flex flex-col gap-1.5">
			{rows.map((row, index) => (
				<div
					// biome-ignore lint/suspicious/noArrayIndexKey: rows have no identity besides their position while they are typed
					key={index}
					className="grid grid-cols-[150px_minmax(0,1fr)_auto] items-center gap-1.5 @max-[520px]/devices:grid-cols-1"
				>
					<AreaSelect
						label={t("deployShip.writes.folderArea", "Folder {{n}} area", {
							n: index + 1,
						})}
						value={row.purpose}
						options={areas}
						disabled={disabled}
						onChange={(purpose) =>
							onChange(replaceAt(rows, index, { ...row, purpose }))
						}
					/>
					<DvInput
						mono
						disabled={disabled}
						placeholder={t("deployShip.writes.folderPlaceholder", "exports")}
						aria-label={t("deployShip.writes.folderPath", "Folder {{n}} path", {
							n: index + 1,
						})}
						aria-invalid={!folderValid(row)}
						value={row.prefix}
						onChange={(event) =>
							onChange(
								replaceAt(rows, index, { ...row, prefix: event.target.value }),
							)
						}
					/>
					<DvButton
						size="xs"
						variant="ghost"
						iconOnly
						icon={X}
						disabled={disabled}
						aria-label={t(
							"deployShip.writes.removeFolder",
							"Remove folder {{n}}",
							{
								n: index + 1,
							},
						)}
						onClick={() => onChange(removeAt(rows, index))}
					/>
				</div>
			))}
			<DvButton
				size="sm"
				variant="ghost"
				icon={Plus}
				className="self-start"
				disabled={disabled}
				onClick={() => onChange([...rows, { purpose: "files", prefix: "" }])}
			>
				{t("deployShip.writes.addFolder", "Add folder")}
			</DvButton>
		</div>
	);
}

interface BudgetProps {
	value: OfflineWritesConfig;
	disabled: boolean;
	onChange(value: OfflineWritesConfig): void;
}

const whole = (text: string) => Math.max(0, Math.round(Number(text) || 0));

function Budgets({ value, disabled, onChange }: Readonly<BudgetProps>) {
	const { t } = useTranslation("devices");
	const id = useId();
	return (
		<div className="grid grid-cols-2 gap-3 @min-[640px]/devices:grid-cols-4">
			<Field
				id={`${id}-queue`}
				label={t("deployShip.writes.queue", "Waiting changes")}
			>
				<InputWithUnit
					unit="MiB"
					numeric
					inputMode="numeric"
					disabled={disabled}
					value={String(Math.round(value.max_queue_bytes / MIB))}
					onChange={(event) =>
						onChange({
							...value,
							max_queue_bytes: whole(event.target.value) * MIB,
						})
					}
				/>
			</Field>
			<Field
				id={`${id}-count`}
				label={t("deployShip.writes.count", "Changes at most")}
			>
				<DvInput
					numeric
					inputMode="numeric"
					disabled={disabled}
					value={String(value.max_operations)}
					onChange={(event) =>
						onChange({ ...value, max_operations: whole(event.target.value) })
					}
				/>
			</Field>
			<Field
				id={`${id}-age`}
				label={t("deployShip.writes.age", "Oldest change")}
			>
				<InputWithUnit
					unit={t("deployShip.writes.days", "days")}
					numeric
					inputMode="numeric"
					disabled={disabled}
					value={String(Math.round(value.max_age_seconds / DAY))}
					onChange={(event) =>
						onChange({
							...value,
							max_age_seconds: whole(event.target.value) * DAY,
						})
					}
				/>
			</Field>
			<Field
				id={`${id}-copy`}
				label={t("deployShip.writes.copy", "Local table copy")}
			>
				<InputWithUnit
					unit="GiB"
					numeric
					inputMode="numeric"
					disabled={disabled}
					value={String(Math.round(value.max_mirror_bytes / GIB))}
					onChange={(event) =>
						onChange({
							...value,
							max_mirror_bytes: whole(event.target.value) * GIB,
						})
					}
				/>
			</Field>
		</div>
	);
}

interface ListHeadProps {
	title: string;
	hint: string;
}

function ListHead({ title, hint }: Readonly<ListHeadProps>) {
	return (
		<div className="flex flex-col gap-0.5">
			<span className="text-ui font-medium">{title}</span>
			<span className="text-xs text-muted-foreground">{hint}</span>
		</div>
	);
}

function Selection({ value, disabled, onChange }: Readonly<BudgetProps>) {
	const { t } = useTranslation("devices");
	const issues = writeBufferingIssues(value);
	function setTables(tables: Table[]) {
		onChange({ ...value, tables });
	}
	function setFolders(files: Folder[]) {
		onChange({ ...value, files });
	}
	return (
		<div className="flex flex-col gap-3">
			<div className="flex flex-col gap-1.5">
				<ListHead
					title={t("deployShip.writes.tables", "Tables")}
					hint={t("deployShip.writes.tablesHint", "Area · table · key column")}
				/>
				<TableRows
					rows={value.tables}
					disabled={disabled}
					onChange={setTables}
				/>
			</div>
			<div className="flex flex-col gap-1.5">
				<ListHead
					title={t("deployShip.writes.folders", "Folders")}
					hint={t("deployShip.writes.foldersHint", "Area · folder inside it")}
				/>
				<FolderRows
					rows={value.files}
					disabled={disabled}
					onChange={setFolders}
				/>
			</div>
			{issues.length ? (
				<ul
					role="alert"
					className="flex flex-col gap-0.5 text-xs text-critical"
				>
					{issues.map((issue) => (
						<li key={issue}>{ISSUE_COPY[issue](t)}</li>
					))}
				</ul>
			) : null}
			<Budgets value={value} disabled={disabled} onChange={onChange} />
			<p className="text-xs text-muted-foreground">
				{t(
					"deployShip.writes.budgetHint",
					"Beyond these budgets the device refuses new changes while offline, and the service sees the error.",
				)}
			</p>
		</div>
	);
}

export interface WriteBufferingProps {
	/** `null` = off. */
	value: OfflineWritesConfig | null;
	onChange(value: OfflineWritesConfig | null): void;
	/** The services' max instances: buffering needs exactly one. */
	maxInstances: number;
	disabled?: boolean;
}

function InstanceNote({
	maxInstances,
}: Pick<WriteBufferingProps, "maxInstances">) {
	const { t } = useTranslation("devices");
	const needs = t(
		"deployShip.writes.needsOne",
		"Changes wait on the device and reach the cloud when it's back. Needs 1 instance.",
	);
	const drops = t(
		"deployShip.writes.dropsInstances",
		"Max instances drops from {{count, number}} to 1.",
		{ count: maxInstances },
	);
	return (
		<p className="flex items-start gap-1.5 text-ui text-ink-2">
			<Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
			<span>{maxInstances > 1 ? [needs, drops].join(" ") : needs}</span>
		</p>
	);
}

export function WriteBufferingFields(props: WriteBufferingProps) {
	const { value, onChange, maxInstances } = props;
	const disabled = props.disabled === true;
	const { t } = useTranslation("devices");
	const id = useId();
	function toggle(on: boolean) {
		onChange(on ? DEFAULT_WRITE_BUFFERING : null);
	}
	return (
		<div className="flex flex-col gap-3" data-write-buffering="">
			<SwitchField
				id={`${id}-on`}
				checked={value !== null}
				disabled={disabled}
				onCheckedChange={toggle}
			>
				{t(
					"deployShip.writes.switch",
					"Keep accepting changes when the internet drops",
				)}
			</SwitchField>
			{value ? (
				<>
					<InstanceNote maxInstances={maxInstances} />
					<Selection value={value} disabled={disabled} onChange={onChange} />
				</>
			) : null}
		</div>
	);
}
