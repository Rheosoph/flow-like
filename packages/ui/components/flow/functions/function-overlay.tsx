"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronRightIcon,
	DatabaseIcon,
	MaximizeIcon,
	PlusIcon,
	SaveIcon,
	SquareFunctionIcon,
	Trash2Icon,
	TriangleAlertIcon,
} from "lucide-react";
import type { JSX } from "react";
import {
	type RefObject,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import { IVariableType } from "../../../lib";
import {
	type IBoard,
	type ILayer,
	type ILayerCache,
	ILayerCacheScope,
	IPinType,
} from "../../../lib/schema/flow/board";
import {
	Badge,
	Button,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../../ui";
import { normalizeCategory } from "../category-tree";
import {
	CUSTOM_FOLDER,
	FolderPicker,
	ROOT_FOLDER,
	resolveFolder,
} from "../folder-picker";
import {
	CacheSettings,
	type PinEdit,
	PinList,
	usePinEditor,
} from "../layer-editing-menu";
import { OverlayWindow } from "../overlay-window";
import { typeToColor } from "../utils";

const DEFAULT_CACHE: ILayerCache = {
	enabled: false,
	prefix: "",
	ttl_seconds: null,
	scope: ILayerCacheScope.App,
};

const buildCache = (layer: ILayer): ILayerCache => ({
	...DEFAULT_CACHE,
	...(layer.cache ?? {}),
});

export interface IFunctionOverlayProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** The function layer being edited. */
	layer: ILayer;
	/** How many Call nodes reference this function, already computed by the panel. */
	calls: number;
	/** Existing function folder paths, for the folder picker. May be empty. */
	folders: string[];
	boardRef?: RefObject<IBoard | undefined>;
	/** Persists name, category, pins and cache in one command. */
	onApply: (updated: ILayer) => Promise<void>;
	onDelete: () => void;
	/** Navigate the canvas into this function layer. */
	onOpenLayer: () => void;
}

export function FunctionOverlay({
	open,
	onOpenChange,
	layer,
	calls,
	folders,
	boardRef,
	onApply,
	onDelete,
	onOpenLayer,
}: Readonly<IFunctionOverlayProps>): JSX.Element {
	const { t } = useTranslation("flow");

	const {
		inputs,
		outputs,
		reset: resetPins,
		editPin,
		addPin,
		removePin,
		movePin,
		reorderByIds,
		buildPins,
	} = usePinEditor(layer, boardRef);

	const [name, setName] = useState(layer.name);
	const [cache, setCache] = useState<ILayerCache>(() => buildCache(layer));
	const [folder, setFolder] = useState<string>(ROOT_FOLDER);
	const [customFolder, setCustomFolder] = useState("");
	const [tab, setTab] = useState<"inputs" | "outputs" | "caching">("inputs");
	const [confirmDelete, setConfirmDelete] = useState(false);
	const [saving, setSaving] = useState(false);

	useEffect(() => {
		if (!open) return;
		resetPins();
		setName(layer.name);
		setCache(buildCache(layer));
		const current = normalizeCategory(layer.category);
		setFolder(current ?? ROOT_FOLDER);
		setCustomFolder(current ?? "");
		setTab("inputs");
		setConfirmDelete(false);
		setSaving(false);
	}, [open, layer, resetPins]);

	const folderOptions = useMemo(() => {
		const current = normalizeCategory(layer.category);
		const unique = new Set(
			folders
				.map((path) => normalizeCategory(path))
				.filter(Boolean) as string[],
		);
		if (current) unique.add(current);
		return [...unique].sort((a, b) => a.localeCompare(b));
	}, [folders, layer.category]);

	const nextCategory = useMemo(
		() => resolveFolder(folder, customFolder),
		[folder, customFolder],
	);

	const trimmedName = name.trim();

	const applyChanges = useCallback(async () => {
		setSaving(true);
		try {
			const updated: ILayer = {
				...layer,
				name: trimmedName === "" ? layer.name : trimmedName,
				category: nextCategory ?? null,
				pins: buildPins() as unknown as ILayer["pins"],
				cache: { ...cache, prefix: (cache.prefix ?? "").trim() },
			};
			await onApply(updated);
			onOpenChange(false);
		} finally {
			setSaving(false);
		}
	}, [
		layer,
		trimmedName,
		nextCategory,
		buildPins,
		cache,
		onApply,
		onOpenChange,
	]);

	return (
		<OverlayWindow
			open={open}
			onOpenChange={onOpenChange}
			title={t("editFunction", "Edit function")}
			icon={
				<SquareFunctionIcon
					className="size-4"
					style={{ color: "var(--pin-fn-ref)" }}
				/>
			}
			name={name}
			onNameChange={setName}
			nameLabel={t("functionName", "Function name")}
			badge={
				<Badge variant="secondary" className="shrink-0 font-normal">
					{t("callsCallSites", "{{calls}} call sites", { calls })}
				</Badge>
			}
			actions={
				<Button
					variant="ghost"
					size="sm"
					className="gap-1.5"
					onClick={onOpenLayer}
				>
					<MaximizeIcon className="size-3.5" />
					{t("openLayer", "Open layer")}
				</Button>
			}
			rail={
				<>
					<div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
						<div className="space-y-2">
							<p className="text-xs font-medium text-muted-foreground">
								{t("onTheCanvas", "On the canvas")}
							</p>
							<CallNodePreview
								name={trimmedName === "" ? layer.name : trimmedName}
								inputs={inputs}
								outputs={outputs}
								cached={Boolean(cache.enabled)}
							/>
							<p className="text-xs text-muted-foreground">
								{t(
									"everyCallNodeForThisFunctionLooksLikeThis",
									"Every Call node for this function looks like this.",
								)}
							</p>
						</div>

						<CallSitesNotice calls={calls} />
					</div>

					<DangerZone
						confirming={confirmDelete}
						calls={calls}
						onAsk={() => setConfirmDelete(true)}
						onCancel={() => setConfirmDelete(false)}
						onConfirm={onDelete}
					/>
				</>
			}
			footer={
				<>
					<span className="truncate text-xs text-muted-foreground">
						{t(
							"inputcountInOutputcountOut",
							"{{inputCount}} in · {{outputCount}} out",
							{
								inputCount: inputs.length,
								outputCount: outputs.length,
							},
						)}
					</span>
					<div className="flex items-center gap-2">
						<Button variant="ghost" onClick={() => onOpenChange(false)}>
							{t("cancel", "Cancel")}
						</Button>
						<Button className="gap-2" onClick={applyChanges} disabled={saving}>
							<SaveIcon className="h-4 w-4" />
							{t("saveFunction", "Save function")}
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
				hint={t(
					"useToNestFoldersItOnlyChangesWhereTheFunctionIsFiled",
					"Use “/” to nest folders. It only changes where the function is filed, never how it runs.",
				)}
			/>

			<Tabs
				value={tab}
				onValueChange={(value) => setTab(value as typeof tab)}
				className="flex min-h-0 flex-1 flex-col gap-0 px-4 pb-4"
			>
				<TabsList className="flex w-full">
					<TabsTrigger value="inputs" className="min-w-0 flex-1">
						<span className="truncate">{t("inputs", "Inputs")}</span>
					</TabsTrigger>
					<TabsTrigger value="outputs" className="min-w-0 flex-1">
						<span className="truncate">{t("outputs", "Outputs")}</span>
					</TabsTrigger>
					<TabsTrigger value="caching" className="min-w-0 flex-1 gap-1.5">
						<DatabaseIcon className="hidden h-3.5 w-3.5 shrink-0 lg:block" />
						<span className="truncate">{t("caching", "Caching")}</span>
					</TabsTrigger>
				</TabsList>

				<TabsContent
					value="inputs"
					className="mt-3 min-h-0 flex-1 space-y-2 overflow-y-auto overflow-x-hidden"
				>
					<div className="flex justify-end">
						<Button
							size="sm"
							className="gap-2"
							onClick={() => addPin(IPinType.Input)}
						>
							<PlusIcon className="h-4 w-4" />
							{t("addInputPin", "Add Input Pin")}
						</Button>
					</div>
					<PinList
						items={inputs}
						onEdit={editPin}
						onMoveUp={(id) => movePin(id, "up")}
						onMoveDown={(id) => movePin(id, "down")}
						onRemove={removePin}
						onReorder={reorderByIds}
					/>
				</TabsContent>

				<TabsContent
					value="outputs"
					className="mt-3 min-h-0 flex-1 space-y-2 overflow-y-auto overflow-x-hidden"
				>
					<div className="flex justify-end">
						<Button
							size="sm"
							className="gap-2"
							onClick={() => addPin(IPinType.Output)}
						>
							<PlusIcon className="h-4 w-4" />
							{t("addOutputPin", "Add Output Pin")}
						</Button>
					</div>
					<PinList
						items={outputs}
						onEdit={editPin}
						onMoveUp={(id) => movePin(id, "up")}
						onMoveDown={(id) => movePin(id, "down")}
						onRemove={removePin}
						onReorder={reorderByIds}
					/>
				</TabsContent>

				<TabsContent
					value="caching"
					className="mt-3 min-h-0 flex-1 overflow-y-auto pr-1"
				>
					<CacheSettings cache={cache} onChange={setCache} />
				</TabsContent>
			</Tabs>
		</OverlayWindow>
	);
}

function PinDot({ pin }: Readonly<{ pin: PinEdit }>) {
	const color = typeToColor(pin.data_type);
	if (pin.data_type === IVariableType.Execution) {
		return (
			<ChevronRightIcon
				className="size-3 shrink-0"
				style={{ color }}
				strokeWidth={3}
			/>
		);
	}
	return (
		<span
			className="size-2 shrink-0 rounded-full"
			style={{ backgroundColor: color }}
		/>
	);
}

function CallNodePreview({
	name,
	inputs,
	outputs,
	cached,
}: Readonly<{
	name: string;
	inputs: PinEdit[];
	outputs: PinEdit[];
	cached: boolean;
}>) {
	const { t } = useTranslation("flow");
	const rows = Math.max(inputs.length, outputs.length);

	return (
		<div className="overflow-hidden rounded-md border border-border bg-card shadow-floating">
			<div
				className="flex items-center gap-1.5 border-b border-border px-2 py-1.5"
				style={{
					backgroundImage:
						"linear-gradient(to right, var(--card), color-mix(in oklab, var(--pin-fn-ref) 60%, var(--card)))",
				}}
			>
				<SquareFunctionIcon className="size-3 shrink-0" />
				<span className="truncate text-[0.7rem] font-medium leading-none">
					{name}
				</span>
				{cached && (
					<DatabaseIcon
						className="ml-auto size-3 shrink-0"
						aria-label={t("cached", "Cached")}
					/>
				)}
			</div>

			{rows === 0 ? (
				<p className="px-2 py-4 text-center text-[0.7rem] text-muted-foreground">
					{t("noPinsYet", "No pins yet.")}
				</p>
			) : (
				<div className="flex flex-row gap-3 px-2 py-2">
					<div className="flex min-w-0 flex-1 flex-col gap-1.5">
						{inputs.map((pin) => (
							<div
								key={pin.id}
								className="flex items-center gap-1.5 motion-safe:transition-colors"
							>
								<PinDot pin={pin} />
								<span className="truncate text-[0.65rem] leading-none text-foreground">
									{pin.friendly_name || pin.name}
								</span>
							</div>
						))}
					</div>
					<div className="flex min-w-0 flex-1 flex-col items-end gap-1.5">
						{outputs.map((pin) => (
							<div
								key={pin.id}
								className="flex items-center gap-1.5 motion-safe:transition-colors"
							>
								<span className="truncate text-[0.65rem] leading-none text-foreground">
									{pin.friendly_name || pin.name}
								</span>
								<PinDot pin={pin} />
							</div>
						))}
					</div>
				</div>
			)}
		</div>
	);
}

function CallSitesNotice({ calls }: Readonly<{ calls: number }>) {
	const { t } = useTranslation("flow");

	if (calls === 0) {
		return (
			<div className="rounded-md border border-border/60 bg-background/60 p-3 text-xs text-muted-foreground">
				{t(
					"nothingCallsThisFunctionYetSoTheSignatureIsFreeToChange",
					"Nothing calls this function yet, so the signature is free to change.",
				)}
			</div>
		);
	}

	return (
		<div className="flex gap-2 rounded-md border border-border/60 bg-background/60 p-3 text-xs">
			<TriangleAlertIcon className="size-4 shrink-0 text-primary" />
			<span className="text-muted-foreground">
				{t(
					"callsCallNodesUseThisFunctionAddingRemovingOrRetypingAPinChangesEveryOneOfThemAndDropsTheConnectionsOnPinsThatDisappear",
					"{{calls}} Call nodes use this function. Adding, removing or retyping a pin changes every one of them, and drops the connections on pins that disappear.",
					{ calls },
				)}
			</span>
		</div>
	);
}

function DangerZone({
	confirming,
	calls,
	onAsk,
	onCancel,
	onConfirm,
}: Readonly<{
	confirming: boolean;
	calls: number;
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
					{t("deleteFunction", "Delete function")}
				</Button>
			</div>
		);
	}

	return (
		<div className="shrink-0 space-y-3 border-t border-destructive/40 bg-destructive/10 p-4">
			<p className="text-xs text-muted-foreground">
				{calls === 0
					? t(
							"thisFunctionIsNotCalledAnywhereDeletingItRemovesItsNodesToo",
							"This function is not called anywhere. Deleting it removes its nodes too.",
						)
					: t(
							"callsCallNodesWillBeLeftUnresolvedTheyStayOnTheirBoardsButCanNoLongerRun",
							"{{calls}} Call nodes will be left unresolved — they stay on their boards but can no longer run.",
							{ calls },
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
