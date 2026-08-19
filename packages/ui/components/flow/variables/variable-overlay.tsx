"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BracesIcon,
	SaveIcon,
	Trash2Icon,
	TriangleAlertIcon,
	WandIcon,
} from "lucide-react";
import type { JSX } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	Button,
	Label,
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
	SelectValue,
	Separator,
	Switch,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../../../components/ui";
import type { IVariable } from "../../../lib/schema/flow/board";
import { IVariableType } from "../../../lib/schema/flow/node";
import { IValueType } from "../../../lib/schema/flow/pin";
import { convertJsonToUint8Array } from "../../../lib/uint8";
import { cn } from "../../../lib/utils";
import { normalizeCategory } from "../category-tree";
import {
	CUSTOM_FOLDER,
	FolderPicker,
	ROOT_FOLDER,
	resolveFolder,
} from "../folder-picker";
import { OverlayWindow } from "../overlay-window";
import { TOKEN_GLYPH, tokenColor, tokenInk } from "../token-board/model";
import { typeToColor } from "../utils";
import { VariablesMenuEdit } from "./variables-menu-edit";

const DATA_TYPES = [
	IVariableType.Boolean,
	IVariableType.Date,
	IVariableType.Float,
	IVariableType.Integer,
	IVariableType.Generic,
	IVariableType.PathBuf,
	IVariableType.String,
	IVariableType.Struct,
	IVariableType.Byte,
];

const VALUE_TYPES = [
	IValueType.Normal,
	IValueType.Array,
	IValueType.HashSet,
	IValueType.HashMap,
];

export function defaultValueFromType(
	valueType: IValueType,
	variableType: IVariableType,
) {
	if (valueType === IValueType.Array) return [];
	if (valueType === IValueType.HashSet) return new Set();
	if (valueType === IValueType.HashMap) return new Map();
	switch (variableType) {
		case IVariableType.Boolean:
			return false;
		case IVariableType.Date:
			return new Date().toISOString();
		case IVariableType.Float:
			return 0.0;
		case IVariableType.Integer:
			return 0;
		case IVariableType.PathBuf:
			return "";
		case IVariableType.String:
			return "";
		case IVariableType.Struct:
			return {};
		default:
			return null;
	}
}

export interface IVariableOverlayProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	variable: IVariable;
	/** Board scope, or the local scope of the function layer you are standing in. */
	scope: "board" | "local";
	/** How many Get/Set nodes reference it, already computed by the panel. */
	uses: number;
	/** Existing variable folder paths, for the folder picker. */
	folders: string[];
	refs?: Record<string, string>;
	onApply: (variable: IVariable, scope: "board" | "local") => Promise<void>;
	onDelete: (variable: IVariable, scope: "board" | "local") => void;
}

/**
 * The variable editor, as a window.
 *
 * It carries the same fields the right-edge drawer did — name, folder, type and
 * container, the three flags, the per-type default value and the struct schema —
 * but it no longer covers the canvas, and it shows what the variable becomes on
 * the graph while you edit it.
 */
export function VariableOverlay({
	open,
	onOpenChange,
	variable,
	scope,
	uses,
	folders,
	refs,
	onApply,
	onDelete,
}: Readonly<IVariableOverlayProps>): JSX.Element {
	const { t } = useTranslation("flow");
	const [draft, setDraft] = useState<IVariable>(variable);
	const [folder, setFolder] = useState<string>(ROOT_FOLDER);
	const [customFolder, setCustomFolder] = useState("");
	const [tab, setTab] = useState<"value" | "type" | "access">("value");
	const [confirmDelete, setConfirmDelete] = useState(false);
	const [saving, setSaving] = useState(false);

	const editable = variable.editable !== false;

	useEffect(() => {
		if (!open) return;
		setDraft(variable);
		const current = normalizeCategory(variable.category);
		setFolder(current ?? ROOT_FOLDER);
		setCustomFolder(current ?? "");
		setTab("value");
		setConfirmDelete(false);
		setSaving(false);
	}, [open, variable]);

	const folderOptions = useMemo(() => {
		const current = normalizeCategory(variable.category);
		const unique = new Set(
			folders
				.map((path) => normalizeCategory(path))
				.filter(Boolean) as string[],
		);
		if (current) unique.add(current);
		return [...unique].sort((a, b) => a.localeCompare(b));
	}, [folders, variable.category]);

	const patch = useCallback((next: Partial<IVariable>) => {
		setDraft((old) => ({ ...old, ...next }));
	}, []);

	const applyChanges = useCallback(async () => {
		setSaving(true);
		try {
			const name = draft.name.trim();
			await onApply(
				{
					...draft,
					name: name === "" ? variable.name : name,
					category: resolveFolder(folder, customFolder),
				},
				scope,
			);
			onOpenChange(false);
		} finally {
			setSaving(false);
		}
	}, [
		draft,
		folder,
		customFolder,
		onApply,
		onOpenChange,
		scope,
		variable.name,
	]);

	const containerLabel = (valueType: IValueType) => {
		switch (valueType) {
			case IValueType.Normal:
				return t("single", "Single");
			case IValueType.Array:
				return t("array", "Array");
			case IValueType.HashSet:
				return t("set", "Set");
			default:
				return t("map", "Map");
		}
	};

	return (
		<OverlayWindow
			open={open}
			onOpenChange={onOpenChange}
			title={t("editVariable", "Edit Variable")}
			icon={
				<span
					className="flex size-5 items-center justify-center rounded-[3px] font-mono text-[11px]"
					style={{
						backgroundColor: tokenColor(draft.data_type),
						color: tokenInk(draft.data_type),
					}}
				>
					{TOKEN_GLYPH[draft.data_type] ?? "?"}
				</span>
			}
			name={draft.name}
			onNameChange={(name) => patch({ name })}
			nameLabel={t("variableName", "Variable Name")}
			nameDisabled={!editable}
			badge={
				<span className="shrink-0 font-mono text-xs text-muted-foreground">
					{uses === 0
						? t("noReferences", "no references")
						: t("usedByCountNodes", "used by {{count}} nodes", { count: uses })}
				</span>
			}
			rail={
				<>
					<div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
						<div className="space-y-2">
							<p className="text-xs font-medium text-muted-foreground">
								{t("onTheCanvas", "On the canvas")}
							</p>
							<VariableNodePreview variable={draft} operation="get" />
							<VariableNodePreview variable={draft} operation="set" />
							<p className="text-xs text-muted-foreground">
								{t(
									"draggingThisVariableOntoTheCanvasPlacesOneOfThese",
									"Dragging this variable onto the canvas places one of these.",
								)}
							</p>
						</div>

						{uses > 0 && (
							<div className="flex gap-2 rounded-md border border-border/60 bg-background/60 p-3 text-xs">
								<TriangleAlertIcon className="size-4 shrink-0 text-primary" />
								<span className="text-muted-foreground">
									{t(
										"countNodesReadOrWriteThisChangingItsTypeRetypesEveryOneOfThem",
										"{{count}} nodes read or write this. Changing its type retypes every one of them.",
										{ count: uses },
									)}
								</span>
							</div>
						)}
					</div>

					{editable && (
						<VariableDangerZone
							confirming={confirmDelete}
							uses={uses}
							onAsk={() => setConfirmDelete(true)}
							onCancel={() => setConfirmDelete(false)}
							onConfirm={() => onDelete(variable, scope)}
						/>
					)}
				</>
			}
			footer={
				<>
					<span className="truncate font-mono text-xs text-muted-foreground">
						{draft.value_type === IValueType.Normal
							? draft.data_type
							: `${containerLabel(draft.value_type)}<${draft.data_type}>`}
					</span>
					<div className="flex items-center gap-2">
						<Button variant="ghost" onClick={() => onOpenChange(false)}>
							{t("cancel", "Cancel")}
						</Button>
						<Button
							className="gap-2"
							onClick={applyChanges}
							disabled={saving || !editable}
						>
							<SaveIcon className="h-4 w-4" />
							{t("saveVariable", "Save variable")}
						</Button>
					</div>
				</>
			}
		>
			<FolderPicker
				folder={folder}
				onFolderChange={(value) => {
					setFolder(value);
					if (value === CUSTOM_FOLDER) return;
					setCustomFolder(value === ROOT_FOLDER ? "" : value);
				}}
				customFolder={customFolder}
				onCustomFolderChange={setCustomFolder}
				options={folderOptions}
				disabled={!editable}
				hint={t(
					"useToCreateNestedFoldersLeaveEmptyForToplevel",
					"Use “/” to create nested folders. Leave empty for top-level.",
				)}
			/>

			{!editable && (
				<p className="border-b border-border/40 bg-muted/40 px-4 py-2 text-xs text-muted-foreground">
					{t(
						"readOnlyProvidedByTheRuntimeNothingHereCanBeChanged",
						"Read-only — provided by the runtime. Nothing here can be changed.",
					)}
				</p>
			)}

			<Tabs
				value={tab}
				onValueChange={(value) => setTab(value as typeof tab)}
				className="flex min-h-0 flex-1 flex-col gap-0 px-4 pb-4"
			>
				<TabsList className="flex w-full">
					<TabsTrigger value="value" className="min-w-0 flex-1">
						<span className="truncate">
							{t("defaultValue", "Default value")}
						</span>
					</TabsTrigger>
					<TabsTrigger value="type" className="min-w-0 flex-1">
						<span className="truncate">
							{t("variableType", "Variable Type")}
						</span>
					</TabsTrigger>
					<TabsTrigger value="access" className="min-w-0 flex-1">
						<span className="truncate">{t("access", "Access")}</span>
					</TabsTrigger>
				</TabsList>

				<TabsContent
					value="value"
					className="mt-2 min-h-0 flex-1 overflow-y-auto overflow-x-hidden pr-2"
				>
					<TabPane>
						{draft.exposed ? (
							<p className="rounded-md border border-border/60 bg-muted/40 p-3 text-xs text-muted-foreground">
								{t(
									"exposedVariablesTakeTheirValueFromTheAppsConfigurationTab",
									"Exposed variables take their value from the app's configuration tab, so there is no default to edit here.",
								)}
							</p>
						) : (
							<>
								<VariablesMenuEdit
									key={`${draft.value_type}-${draft.data_type}-${draft.secret}`}
									disabled={!editable}
									variable={draft}
									refs={refs}
									updateVariable={async (updated) =>
										patch({ default_value: updated.default_value })
									}
								/>
								{draft.data_type === IVariableType.Struct && (
									<>
										<Separator />
										<StructSchemaEditor
											variable={draft}
											refs={refs}
											onSchemaChange={(schema) => patch({ schema })}
										/>
									</>
								)}
							</>
						)}
					</TabPane>
				</TabsContent>

				<TabsContent
					value="type"
					className="mt-2 min-h-0 flex-1 overflow-y-auto overflow-x-hidden pr-2"
				>
					<TabPane>
						<div className="space-y-2">
							<div className="flex flex-row gap-2">
								<Select
									value={draft.data_type}
									disabled={!editable}
									onValueChange={(value) =>
										patch({
											data_type: value as IVariableType,
											default_value: convertJsonToUint8Array(
												defaultValueFromType(
													draft.value_type,
													value as IVariableType,
												),
											),
										})
									}
								>
									<SelectTrigger id="var_type" className="flex-1">
										<SelectValue placeholder={t("dataType", "Data Type")} />
									</SelectTrigger>
									<SelectContent>
										<SelectGroup>
											<SelectLabel>{t("dataType", "Data Type")}</SelectLabel>
											{DATA_TYPES.map((type) => (
												<SelectItem key={type} value={type}>
													<div className="flex items-center gap-2">
														<div
															className="size-2 rounded-full"
															style={{ backgroundColor: typeToColor(type) }}
														/>
														<span>{type}</span>
													</div>
												</SelectItem>
											))}
										</SelectGroup>
									</SelectContent>
								</Select>
								<Select
									value={draft.value_type}
									disabled={!editable}
									onValueChange={(value) =>
										patch({
											value_type: value as IValueType,
											default_value: convertJsonToUint8Array(
												defaultValueFromType(
													value as IValueType,
													draft.data_type,
												),
											),
										})
									}
								>
									<SelectTrigger className="w-40 shrink-0">
										<SelectValue placeholder={t("valueType", "Value Type")} />
									</SelectTrigger>
									<SelectContent>
										<SelectGroup>
											<SelectLabel>{t("valueType", "Value Type")}</SelectLabel>
											{VALUE_TYPES.map((valueType) => (
												<SelectItem key={valueType} value={valueType}>
													{containerLabel(valueType)}
												</SelectItem>
											))}
										</SelectGroup>
									</SelectContent>
								</Select>
							</div>
							<small className="block text-[0.8rem] text-muted-foreground">
								{t(
									"changingEitherResetsTheDefaultValueToTheNewTypesEmptyValue",
									"Changing either resets the default value to the new type's empty value.",
								)}
							</small>
						</div>
					</TabPane>
				</TabsContent>

				<TabsContent
					value="access"
					className="mt-2 min-h-0 flex-1 overflow-y-auto overflow-x-hidden pr-2"
				>
					<TabPane>
						<FlagRow
							id="exposed"
							checked={draft.exposed}
							disabled={!editable}
							onChange={(exposed) => patch({ exposed })}
							label={t("isExposed", "Is Exposed?")}
							hint={t(
								"ifYouExposeAVariableItWillBeVisibleInTheConfigurationTabOfYourApp",
								"If you expose a variable it will be visible in the configuration tab of your App.",
							)}
						/>
						<FlagRow
							id="secret"
							checked={draft.secret}
							disabled={!editable}
							onChange={(secret) => patch({ secret })}
							label={t("isSecret", "Is Secret?")}
							hint={t(
								"aSecretVariableWillBeCoveredForInputEgPasswords",
								"A secret variable will be covered for input (e.g passwords)",
							)}
						/>
						<FlagRow
							id="runtime_configured"
							checked={draft.runtime_configured ?? false}
							disabled={!editable}
							onChange={(runtime_configured) => patch({ runtime_configured })}
							label={t("runtimeConfigured", "Runtime Configured?")}
							hint={t(
								"runtimeConfiguredVariablesAreSetPeruserLocallyTheyAreNeverStoredInTheFlowItself",
								"Runtime configured variables are set per-user locally. They are never stored in the flow itself.",
							)}
						/>
					</TabPane>
				</TabsContent>
			</Tabs>
		</OverlayWindow>
	);
}

/**
 * One measure and one vertical rhythm for every tab, so fields stop floating in
 * a 600 px void and hint text stops running the full width of the pane.
 */
function TabPane({ children }: Readonly<{ children: React.ReactNode }>) {
	return <div className="max-w-xl space-y-6 py-4">{children}</div>;
}

function FlagRow({
	id,
	checked,
	disabled,
	onChange,
	label,
	hint,
}: Readonly<{
	id: string;
	checked: boolean;
	disabled?: boolean;
	onChange: (next: boolean) => void;
	label: string;
	hint: string;
}>) {
	return (
		<div className="flex flex-col gap-1">
			<div className="flex items-center space-x-2">
				<Switch
					checked={checked}
					disabled={disabled}
					onCheckedChange={onChange}
					id={id}
				/>
				<Label htmlFor={id}>{label}</Label>
			</div>
			<small className="max-w-prose text-[0.8rem] text-muted-foreground">
				{hint}
			</small>
		</div>
	);
}

/** The Get/Set node this variable becomes on the graph. */
function VariableNodePreview({
	variable,
	operation,
}: Readonly<{ variable: IVariable; operation: "get" | "set" }>) {
	const color = typeToColor(variable.data_type);
	const isSet = operation === "set";

	return (
		<div className="overflow-hidden rounded-md border border-border bg-card shadow-floating">
			<div
				className="flex items-center gap-1.5 border-b border-border px-2 py-1.5"
				style={{
					backgroundImage: `linear-gradient(to right, var(--card), color-mix(in oklab, ${color} 55%, var(--card)))`,
				}}
			>
				<span className="truncate font-mono text-[0.7rem] font-medium leading-none">
					{isSet ? "Set" : "Get"} {variable.name}
				</span>
			</div>
			<div className="flex flex-col gap-1.5 px-2 py-2">
				{isSet && (
					<div className="flex items-center justify-between gap-3 text-[0.65rem] text-muted-foreground">
						<span className="flex items-center gap-1.5">
							<span className="text-foreground">›</span>exec
						</span>
						<span className="flex items-center gap-1.5">
							exec<span className="text-foreground">›</span>
						</span>
					</div>
				)}
				<div
					className={cn(
						"flex items-center gap-1.5 text-[0.65rem] text-foreground",
						!isSet && "justify-end",
					)}
				>
					{isSet && (
						<span
							className="size-2 shrink-0 rounded-full"
							style={{ backgroundColor: color }}
						/>
					)}
					<span className="truncate">{variable.data_type}</span>
					{!isSet && (
						<span
							className="size-2 shrink-0 rounded-full"
							style={{ backgroundColor: color }}
						/>
					)}
				</div>
			</div>
		</div>
	);
}

function VariableDangerZone({
	confirming,
	uses,
	onAsk,
	onCancel,
	onConfirm,
}: Readonly<{
	confirming: boolean;
	uses: number;
	onAsk: () => void;
	onCancel: () => void;
	onConfirm: () => void;
}>) {
	const { t } = useTranslation("flow");

	if (!confirming) {
		return (
			<div className="shrink-0 border-t border-border/40 p-4">
				<Button
					variant="ghost"
					size="sm"
					className="w-full gap-2 text-destructive hover:bg-destructive/10 hover:text-destructive"
					onClick={onAsk}
				>
					<Trash2Icon className="size-3.5" />
					{t("deleteVariable", "Delete variable")}
				</Button>
			</div>
		);
	}

	return (
		<div className="shrink-0 space-y-3 border-t border-destructive/40 bg-destructive/10 p-4">
			<p className="text-xs text-muted-foreground">
				{uses === 0
					? t(
							"nothingReferencesThisVariableTheCanvasWillNotChange",
							"Nothing references this variable. The canvas will not change.",
						)
					: t(
							"countGetSetNodesWillBeLeftUnresolvedTheyStayOnTheBoardButCanNoLongerRun",
							"{{count}} Get/Set nodes will be left unresolved — they stay on the board but can no longer run.",
							{ count: uses },
						)}
			</p>
			<div className="flex items-center gap-2">
				<Button
					variant="secondary"
					size="sm"
					className="flex-1"
					onClick={onCancel}
				>
					{t("cancel", "Cancel")}
				</Button>
				<Button
					variant="destructive"
					size="sm"
					className="flex-1 gap-2"
					onClick={onConfirm}
				>
					<Trash2Icon className="size-3.5" />
					{t("delete", "Delete")}
				</Button>
			</div>
		</div>
	);
}

const EMPTY_STRING_HASH = "16248035215404677707";

const resolveRef = (
	value: string | undefined | null,
	refs: Record<string, string> | undefined,
): string => {
	if (!value) return "";
	if (value === EMPTY_STRING_HASH) return "";
	return refs?.[value] ?? value;
};

export function StructSchemaEditor({
	variable,
	refs,
	onSchemaChange,
}: Readonly<{
	variable: IVariable;
	refs?: Record<string, string>;
	onSchemaChange: (schema: string | null) => void;
}>) {
	const { t } = useTranslation("flow");
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
		if (resolvedSchema) setSchemaJson(resolvedSchema);
	}, [resolvedSchema]);

	const handleGenerateFromExample = useCallback(() => {
		try {
			const parsed = JSON.parse(exampleJson);
			const schemaStr = JSON.stringify(
				generateSchemaFromExample(parsed),
				null,
				2,
			);
			setSchemaJson(schemaStr);
			onSchemaChange(schemaStr);
			setError(null);
		} catch {
			setError(t("invalidJsonExample", "Invalid JSON example"));
		}
	}, [exampleJson, onSchemaChange, t]);

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
				setError(t("invalidJsonSchema", "Invalid JSON schema"));
			}
		},
		[onSchemaChange, t],
	);

	return (
		<div className="flex flex-col gap-2">
			<Label className="flex items-center gap-2">
				<BracesIcon className="w-4 h-4" />
				{t("schema", "Schema")}
			</Label>
			<small className="-mt-1 text-[0.8rem] text-muted-foreground">
				{t(
					"defineAJsonSchemaToEnableFormbasedEditingForThisStruct",
					"Define a JSON schema to enable form-based editing for this struct.",
				)}
			</small>

			<Tabs
				value={schemaMode}
				onValueChange={(v) => setSchemaMode(v as "example" | "schema")}
			>
				<TabsList className="grid w-full grid-cols-2">
					<TabsTrigger value="example" className="gap-1">
						<WandIcon className="w-3 h-3" />
						{t("fromExample", "From Example")}
					</TabsTrigger>
					<TabsTrigger value="schema" className="gap-1">
						<BracesIcon className="w-3 h-3" />
						{t("editSchema", "Edit Schema")}
					</TabsTrigger>
				</TabsList>

				<TabsContent value="example" className="space-y-2">
					<small className="text-[0.8rem] text-muted-foreground">
						{t(
							"pasteAnExampleJsonAndGenerateASchemaAutomatically",
							"Paste an example JSON and generate a schema automatically.",
						)}
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
							placeholder={`{"name": "John", "age": 30}`}
							rows={5}
							className="w-full resize-none bg-transparent px-3 py-2 font-mono text-sm outline-none"
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
						{t("generateSchema", "Generate Schema")}
					</Button>
				</TabsContent>

				<TabsContent value="schema" className="space-y-2">
					<small className="text-[0.8rem] text-muted-foreground">
						{t(
							"editTheJsonSchemaDirectlyLeaveEmptyToDisableFormMode",
							"Edit the JSON schema directly. Leave empty to disable form mode.",
						)}
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
							placeholder={`{"type": "object", "properties": {...}}`}
							rows={8}
							className="w-full resize-none bg-transparent px-3 py-2 font-mono text-sm outline-none"
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
							{t("clearSchema", "Clear Schema")}
						</Button>
					)}
				</TabsContent>
			</Tabs>
		</div>
	);
}

function generateSchemaFromExample(example: unknown): object {
	if (example === null) return { type: "null" };

	if (Array.isArray(example)) {
		return {
			type: "array",
			items: example.length > 0 ? generateSchemaFromExample(example[0]) : {},
		};
	}

	if (typeof example === "object") {
		const properties: Record<string, object> = {};
		const required: string[] = [];
		for (const [key, value] of Object.entries(example)) {
			properties[key] = generateSchemaFromExample(value);
			if (value !== null && value !== undefined) required.push(key);
		}
		return {
			type: "object",
			properties,
			required: required.length > 0 ? required : undefined,
		};
	}

	if (typeof example === "boolean") return { type: "boolean" };
	if (typeof example === "number")
		return Number.isInteger(example) ? { type: "integer" } : { type: "number" };
	if (typeof example === "string") return { type: "string" };
	return {};
}
