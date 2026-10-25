"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import {
	ArrowLeft,
	Boxes,
	CheckCircle2,
	Database,
	HardDrive,
	History,
	Info,
	Lock,
	type LucideIcon,
	MessagesSquare,
	RefreshCw,
	Save,
	Search,
	ShieldAlert,
	ShieldCheck,
	Trash2,
	TriangleAlert,
} from "lucide-react";
import Link from "next/link";
import {
	type CSSProperties,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import {
	BROWSER_STORAGE_CATEGORIES,
	type BrowserStorageCategory,
	type BrowserStorageEntry,
	type BrowserStorageScan,
	type ScanProgress,
	deleteBrowserStorageEntries,
	scanBrowserStorage,
} from "../../../lib/browser-storage/inventory";
import {
	DEFAULT_RETENTION_POLICY,
	type RetentionKind,
	type StorageRetentionPolicy,
	readLastRetentionRun,
	readRetentionPolicy,
	writeRetentionPolicy,
} from "../../../lib/browser-storage/retention";
import { pruneRetentionKind, runIDBCleanup } from "../../../lib/idb-cleanup";
import { cn, humanFileSize } from "../../../lib/utils";
import { historyRegistry } from "../../flow/flow-history";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Checkbox } from "../../ui/checkbox";
import { Input } from "../../ui/input";
import { Skeleton } from "../../ui/skeleton";
import { formatStorageDate } from "./format";
import { RetentionCard } from "./retention-card";
import { type BoardNames, useBoardNames } from "./use-board-names";
import { type StorageCopy, useStorageCopy } from "./use-storage-copy";

type SettingsT = TFunction<"settings">;

const CATEGORY_META: Readonly<
	Record<BrowserStorageCategory, { icon: LucideIcon; color: string }>
> = {
	boards: { icon: History, color: "var(--fl-chat-chart-1)" },
	activity: { icon: MessagesSquare, color: "var(--fl-chat-chart-2)" },
	caches: { icon: Database, color: "var(--fl-chat-chart-3)" },
	local: { icon: Save, color: "var(--fl-chat-chart-4)" },
	protected: { icon: Lock, color: "var(--fl-chat-chart-5)" },
	other: { icon: Boxes, color: "var(--fl-chat-chart-8)" },
};

const softTone = (color: string): CSSProperties => ({
	color,
	backgroundColor: `color-mix(in oklch, ${color} 14%, transparent)`,
});

const clearBoardHistory = (key: string) => historyRegistry.clearKey(key);

const sumBytes = (entries: readonly BrowserStorageEntry[]) =>
	entries.reduce((sum, entry) => sum + entry.sizeBytes, 0);

interface PendingDelete {
	readonly entries: readonly BrowserStorageEntry[];
	readonly category: BrowserStorageCategory;
}

interface CategoryTotal {
	readonly bytes: number;
	readonly count: number;
}

type CategoryTotals = ReadonlyMap<BrowserStorageCategory, CategoryTotal>;

interface EntryDescription {
	readonly title: string;
	readonly detail: string;
	readonly badge?: string;
}

function categoryTotals(
	entries: readonly BrowserStorageEntry[],
): CategoryTotals {
	const totals = new Map<BrowserStorageCategory, CategoryTotal>();
	for (const entry of entries) {
		const total = totals.get(entry.category) ?? { bytes: 0, count: 0 };
		totals.set(entry.category, {
			bytes: total.bytes + entry.sizeBytes,
			count: total.count + 1,
		});
	}
	return totals;
}

function useCategoryTotals(scan: BrowserStorageScan | undefined) {
	const entries = scan?.entries;
	return useMemo(() => categoryTotals(entries ?? []), [entries]);
}

function usePendingDelete() {
	const [pending, setPending] = useState<PendingDelete | null>(null);
	const close = useCallback(() => setPending(null), []);
	return { pending, request: setPending, close };
}

export function BrowserStoragePage({
	backHref = "/settings",
}: Readonly<{ backHref?: string }>) {
	const { scan, progress, scanning, failed, rescan } = useBrowserStorageScan();
	const retention = useRetentionPolicy(rescan);
	const [activeCategory, setActiveCategory] =
		useState<BrowserStorageCategory>("boards");
	const deletion = usePendingDelete();
	const totals = useCategoryTotals(scan);

	const header = (
		<PageHeader backHref={backHref} scanning={scanning} onRefresh={rescan} />
	);
	if (!scan) {
		return (
			<PageFrame>
				{header}
				<ScanPending failed={failed} progress={progress} onRetry={rescan} />
			</PageFrame>
		);
	}

	return (
		<PageFrame>
			{header}
			<StorageHero
				scan={scan}
				totals={totals}
				cleaning={retention.cleaning}
				onSelectCategory={setActiveCategory}
				onRequestDelete={deletion.request}
				onCleanUp={retention.runNow}
			/>
			<div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_360px]">
				<div className="min-w-0 space-y-5">
					<CategoryTiles
						totals={totals}
						active={activeCategory}
						onSelect={setActiveCategory}
					/>
					<CategoryPanel
						key={activeCategory}
						category={activeCategory}
						entries={scan.entries}
						scanning={scanning}
						onRequestDelete={deletion.request}
					/>
				</div>
				<StorageSidebar
					retention={retention}
					persisted={scan.estimate.persisted}
					onChanged={rescan}
				/>
			</div>
			<DeleteDialog
				pending={deletion.pending}
				onClose={deletion.close}
				onDeleted={rescan}
			/>
		</PageFrame>
	);
}

function StorageSidebar({
	retention,
	persisted,
	onChanged,
}: Readonly<{
	retention: ReturnType<typeof useRetentionPolicy>;
	persisted?: boolean;
	onChanged: () => void;
}>) {
	return (
		<aside className="space-y-5">
			<RetentionCard
				policy={retention.policy}
				lastRunAt={retention.lastRunAt}
				busy={retention.cleaning}
				onPolicyChange={retention.change}
				onRunNow={retention.runNow}
			/>
			<PersistenceCard persisted={persisted} onChanged={onChanged} />
			<LocalOnlyNote />
		</aside>
	);
}

function useBrowserStorageScan() {
	const [scan, setScan] = useState<BrowserStorageScan>();
	const [progress, setProgress] = useState<ScanProgress>();
	const [failed, setFailed] = useState(false);
	const running = useRef<AbortController | undefined>(undefined);

	const rescan = useCallback(() => {
		running.current?.abort();
		const controller = new AbortController();
		running.current = controller;
		const live = () => !controller.signal.aborted;
		setProgress({ done: 0, total: 1 });
		setFailed(false);
		scanBrowserStorage({
			signal: controller.signal,
			onProgress: (next) => live() && setProgress(next),
		})
			.then((result) => live() && setScan(result))
			.catch(() => live() && setFailed(true))
			.finally(() => live() && setProgress(undefined));
	}, []);

	useEffect(() => {
		rescan();
		return () => running.current?.abort();
	}, [rescan]);

	return { scan, progress, scanning: progress !== undefined, failed, rescan };
}

function useRetentionPolicy(onPruned: () => void) {
	const { t } = useTranslation("settings");
	const [policy, setPolicy] = useState<StorageRetentionPolicy>(
		DEFAULT_RETENTION_POLICY,
	);
	const [lastRunAt, setLastRunAt] = useState<number>();
	const [cleaning, setCleaning] = useState(false);

	useEffect(() => {
		setPolicy(readRetentionPolicy());
		setLastRunAt(readLastRetentionRun());
	}, []);

	const removedLabel = useCallback(
		(count: number) =>
			t("storageRemovedOldEntries", {
				count,
				defaultValue_one: "Removed {{count}} old entry",
				defaultValue_other: "Removed {{count}} old entries",
			}),
		[t],
	);
	const reportFailure = useCallback(
		(error: unknown) =>
			toast.error(t("storageCleanupFailed", "Cleanup failed"), {
				description: String(error),
			}),
		[t],
	);

	const change = useCallback(
		async (kind: RetentionKind, days: number | null) => {
			const next = { ...policy, [kind]: days };
			setPolicy(next);
			writeRetentionPolicy(next);
			const saved = t("storageRetentionSaved", "Cleanup setting saved");
			if (days === null) {
				toast.success(saved);
				return;
			}
			setCleaning(true);
			try {
				const removed = await pruneRetentionKind(kind, days);
				toast.success(saved, {
					description: removed > 0 ? removedLabel(removed) : undefined,
				});
				if (removed > 0) onPruned();
			} catch (error) {
				reportFailure(error);
			} finally {
				setCleaning(false);
			}
		},
		[onPruned, policy, removedLabel, reportFailure, t],
	);

	const runNow = useCallback(async () => {
		setCleaning(true);
		try {
			const removed = await runIDBCleanup(policy);
			const count = Object.values(removed).reduce(
				(sum, value) => sum + (value ?? 0),
				0,
			);
			setLastRunAt(readLastRetentionRun());
			toast.success(
				count > 0
					? removedLabel(count)
					: t("storageNothingToClean", "Nothing is old enough to remove"),
			);
			onPruned();
		} catch (error) {
			reportFailure(error);
		} finally {
			setCleaning(false);
		}
	}, [onPruned, policy, removedLabel, reportFailure, t]);

	return { policy, lastRunAt, cleaning, change, runNow };
}

function useEntryDescriber(entries: readonly BrowserStorageEntry[]) {
	const { t } = useTranslation("settings");
	const copy = useStorageCopy();
	const appIds = useMemo(
		() =>
			entries.flatMap((entry) =>
				entry.board?.appId ? [entry.board.appId] : [],
			),
		[entries],
	);
	const names = useBoardNames(appIds);
	return useCallback(
		(entry: BrowserStorageEntry) => describeEntry(entry, copy, names, t),
		[copy, names, t],
	);
}

function PageFrame({ children }: Readonly<{ children: React.ReactNode }>) {
	return (
		<div className="h-full overflow-auto px-1 pb-8">
			<div className="mx-auto max-w-screen-2xl space-y-5">{children}</div>
		</div>
	);
}

function PageHeader({
	backHref,
	scanning,
	onRefresh,
}: Readonly<{ backHref: string; scanning: boolean; onRefresh: () => void }>) {
	const { t } = useTranslation("settings");
	return (
		<div className="flex items-start justify-between gap-4 pt-1">
			<div className="min-w-0">
				<Link
					href={backHref}
					className="mb-2 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
				>
					<ArrowLeft className="size-4" />{" "}
					{t("storageBackToSettings", "Settings")}
				</Link>
				<h1 className="text-3xl font-bold tracking-tight">
					{t("storageTitle", "Storage")}
				</h1>
				<p className="mt-1 text-muted-foreground">
					{t(
						"storageSubtitle",
						"What Flow-Like keeps in this browser, and how to clean it up.",
					)}
				</p>
			</div>
			<Button
				variant="outline"
				size="sm"
				className="shrink-0"
				onClick={onRefresh}
				disabled={scanning}
			>
				<RefreshCw className={cn(scanning && "animate-spin")} />
				{t("storageRefresh", "Refresh")}
			</Button>
		</div>
	);
}

function StorageHero({
	scan,
	totals,
	cleaning,
	onSelectCategory,
	onRequestDelete,
	onCleanUp,
}: Readonly<{
	scan: BrowserStorageScan;
	totals: CategoryTotals;
	cleaning: boolean;
	onSelectCategory: (category: BrowserStorageCategory) => void;
	onRequestDelete: (pending: PendingDelete) => void;
	onCleanUp: () => void;
}>) {
	const { t } = useTranslation("settings");
	const copy = useStorageCopy();
	const total = sumBytes(scan.entries);
	const caches = useMemo(
		() =>
			scan.entries.filter(
				(entry) => entry.category === "caches" && entry.deletable,
			),
		[scan],
	);
	const { usage, quota } = scan.estimate;

	return (
		<section className="relative overflow-hidden rounded-2xl border bg-linear-to-br from-card via-card to-primary/7 p-5 shadow-sm md:p-7">
			<div className="pointer-events-none absolute -right-24 -top-28 size-80 rounded-full bg-primary/10 blur-3xl" />
			<div className="relative space-y-5">
				<div className="flex flex-wrap items-end justify-between gap-4">
					<div className="flex items-start gap-3">
						<div className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-sm">
							<HardDrive className="size-5" />
						</div>
						<div>
							<p className="text-sm font-medium text-muted-foreground">
								{t("storageStoredInBrowser", "Stored in this browser")}
							</p>
							<p className="text-4xl font-semibold tracking-tight tabular-nums md:text-5xl">
								{humanFileSize(total)}
							</p>
							{usage !== undefined && quota ? (
								<p className="mt-1 text-xs text-muted-foreground">
									{t(
										"storageDiskUsage",
										"{{disk}} on disk of {{quota}} the browser allows",
										{ disk: humanFileSize(usage), quota: humanFileSize(quota) },
									)}
								</p>
							) : null}
						</div>
					</div>
					<div className="flex flex-wrap gap-2">
						<Button
							variant="outline"
							size="sm"
							disabled={caches.length === 0}
							onClick={() =>
								onRequestDelete({ entries: caches, category: "caches" })
							}
						>
							<Database />
							{t("storageClearCaches", "Clear caches")}
						</Button>
						<Button size="sm" disabled={cleaning} onClick={onCleanUp}>
							<RefreshCw className={cn(cleaning && "animate-spin")} />
							{t("storageCleanUpNow", "Clean up now")}
						</Button>
					</div>
				</div>

				<UsageBar totals={totals} total={total} />

				<div className="grid gap-x-7 gap-y-3 sm:grid-cols-2 lg:grid-cols-3">
					{BROWSER_STORAGE_CATEGORIES.map((category) => (
						<button
							type="button"
							key={category}
							onClick={() => onSelectCategory(category)}
							className="group flex min-w-0 items-center gap-2 text-left"
						>
							<span
								className="size-2.5 shrink-0 rounded-full"
								style={{ backgroundColor: CATEGORY_META[category].color }}
							/>
							<span className="truncate text-sm text-muted-foreground group-hover:text-foreground">
								{copy.categories[category].label}
							</span>
							<span className="ml-auto text-sm font-semibold tabular-nums">
								{humanFileSize(totals.get(category)?.bytes ?? 0)}
							</span>
						</button>
					))}
				</div>
			</div>
		</section>
	);
}

function UsageBar({
	totals,
	total,
}: Readonly<{ totals: CategoryTotals; total: number }>) {
	if (total <= 0) return <div className="h-2.5 rounded-full bg-muted" />;
	return (
		<div className="flex h-2.5 overflow-hidden rounded-full bg-muted">
			{BROWSER_STORAGE_CATEGORIES.map((category) => {
				const bytes = totals.get(category)?.bytes ?? 0;
				if (bytes <= 0) return null;
				return (
					<div
						key={category}
						className="h-full border-r border-background last:border-r-0"
						style={{
							width: `${Math.max((bytes / total) * 100, 0.75)}%`,
							backgroundColor: CATEGORY_META[category].color,
						}}
					/>
				);
			})}
		</div>
	);
}

function CategoryTiles({
	totals,
	active,
	onSelect,
}: Readonly<{
	totals: CategoryTotals;
	active: BrowserStorageCategory;
	onSelect: (category: BrowserStorageCategory) => void;
}>) {
	const copy = useStorageCopy();
	return (
		<div className="grid grid-cols-2 gap-3 md:grid-cols-3 2xl:grid-cols-6">
			{BROWSER_STORAGE_CATEGORIES.map((category) => (
				<CategoryTile
					key={category}
					category={category}
					label={copy.categories[category].label}
					total={totals.get(category)}
					active={active === category}
					onSelect={onSelect}
				/>
			))}
		</div>
	);
}

function CategoryTile({
	category,
	label,
	total,
	active,
	onSelect,
}: Readonly<{
	category: BrowserStorageCategory;
	label: string;
	total?: CategoryTotal;
	active: boolean;
	onSelect: (category: BrowserStorageCategory) => void;
}>) {
	const { t } = useTranslation("settings");
	const meta = CATEGORY_META[category];
	const Icon = meta.icon;
	return (
		<button
			type="button"
			onClick={() => onSelect(category)}
			aria-pressed={active}
			className={cn(
				"min-w-0 rounded-xl border bg-card p-4 text-left shadow-xs transition-all hover:-translate-y-0.5 hover:shadow-md",
				active && "border-primary ring-2 ring-primary/15",
			)}
		>
			<div
				className="mb-4 flex size-9 items-center justify-center rounded-lg"
				style={softTone(meta.color)}
			>
				<Icon className="size-4.5" />
			</div>
			<p className="truncate text-sm font-medium">{label}</p>
			<div className="mt-1 flex flex-wrap items-baseline justify-between gap-x-2">
				<p className="text-lg font-semibold tabular-nums">
					{humanFileSize(total?.bytes ?? 0)}
				</p>
				<p className="text-xs text-muted-foreground">
					{t("storageItemCount", {
						count: total?.count ?? 0,
						defaultValue_one: "{{count}} item",
						defaultValue_other: "{{count}} items",
					})}
				</p>
			</div>
		</button>
	);
}

/** Keyed by category, so switching categories starts with no selection and no filter. */
function CategoryPanel({
	category,
	entries,
	scanning,
	onRequestDelete,
}: Readonly<{
	category: BrowserStorageCategory;
	entries: readonly BrowserStorageEntry[];
	scanning: boolean;
	onRequestDelete: (pending: PendingDelete) => void;
}>) {
	const { t } = useTranslation("settings");
	const copy = useStorageCopy();
	const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
	const [search, setSearch] = useState("");
	const categoryEntries = useMemo(
		() => entries.filter((entry) => entry.category === category),
		[entries, category],
	);
	const describe = useEntryDescriber(categoryEntries);
	const query = search.trim().toLocaleLowerCase();
	const visibleEntries = useMemo(() => {
		if (!query) return categoryEntries;
		const matches = (entry: BrowserStorageEntry) => {
			const { title, detail } = describe(entry);
			return `${title}\n${detail}`.toLocaleLowerCase().includes(query);
		};
		return categoryEntries.filter(matches);
	}, [categoryEntries, describe, query]);
	const selectedEntries = categoryEntries.filter((entry) =>
		selected.has(entry.id),
	);
	const requestDelete = (list: readonly BrowserStorageEntry[]) =>
		onRequestDelete({ entries: list, category });

	return (
		<Card className="gap-0 overflow-hidden py-0">
			<CardHeader className="gap-3 border-b bg-muted/20 py-4">
				<div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
					<div className="min-w-0">
						<div className="flex items-center gap-2">
							<CardTitle>{copy.categories[category].label}</CardTitle>
							<Badge variant="secondary">{categoryEntries.length}</Badge>
							{scanning && (
								<RefreshCw className="size-3.5 animate-spin text-muted-foreground" />
							)}
						</div>
						<CardDescription className="mt-1 max-w-2xl">
							{copy.categories[category].description}
						</CardDescription>
					</div>
					{selectedEntries.length > 0 && (
						<Button
							variant="destructive"
							size="sm"
							className="shrink-0"
							onClick={() => requestDelete(selectedEntries)}
						>
							<Trash2 />
							{t("storageDeleteSelected", "Delete {{count}} · {{size}}", {
								count: selectedEntries.length,
								size: humanFileSize(sumBytes(selectedEntries)),
							})}
						</Button>
					)}
				</div>
				{categoryEntries.length > 5 && (
					<div className="relative">
						<Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
						<Input
							value={search}
							onChange={(event) => setSearch(event.target.value)}
							placeholder={t("storageSearch", "Filter by name")}
							className="pl-9"
						/>
					</div>
				)}
			</CardHeader>
			<CardContent className="p-0">
				<EntryList
					entries={visibleEntries}
					searching={query.length > 0}
					selected={selected}
					onSelectedChange={setSelected}
					onDelete={(entry) => requestDelete([entry])}
					describe={describe}
				/>
			</CardContent>
		</Card>
	);
}

function EntryList({
	entries,
	searching,
	selected,
	onSelectedChange,
	onDelete,
	describe,
}: Readonly<{
	entries: readonly BrowserStorageEntry[];
	searching: boolean;
	selected: ReadonlySet<string>;
	onSelectedChange: (selected: ReadonlySet<string>) => void;
	onDelete: (entry: BrowserStorageEntry) => void;
	describe: (entry: BrowserStorageEntry) => EntryDescription;
}>) {
	const { t } = useTranslation("settings");
	const deletable = entries.filter((entry) => entry.deletable);

	if (entries.length === 0) {
		return (
			<div className="flex min-h-56 flex-col items-center justify-center px-6 text-center">
				<div className="mb-3 flex size-11 items-center justify-center rounded-full bg-muted text-muted-foreground">
					<CheckCircle2 className="size-5" />
				</div>
				<p className="font-medium">
					{searching
						? t("storageNoMatches", "No matching items")
						: t("storageNothingStored", "Nothing stored here")}
				</p>
				<p className="mt-1 text-sm text-muted-foreground">
					{searching
						? t("storageTryDifferentSearch", "Try a different search.")
						: t("storageCategoryClean", "This category is already clean.")}
				</p>
			</div>
		);
	}

	const toggle = (ids: readonly string[], checked: boolean) => {
		const next = new Set(selected);
		for (const id of ids) {
			if (checked) next.add(id);
			else next.delete(id);
		}
		onSelectedChange(next);
	};

	return (
		<div className="divide-y">
			<div className="grid grid-cols-[28px_minmax(0,1fr)_80px_36px] items-center gap-3 px-4 py-2.5 text-xs font-medium uppercase tracking-wide text-muted-foreground md:grid-cols-[32px_minmax(0,1fr)_100px_150px_40px]">
				<Checkbox
					aria-label={t("storageSelectAll", "Select all deletable items")}
					disabled={deletable.length === 0}
					checked={
						deletable.length > 0 &&
						deletable.every((entry) => selected.has(entry.id))
					}
					onCheckedChange={(checked) =>
						toggle(
							deletable.map((entry) => entry.id),
							checked === true,
						)
					}
				/>
				<span>{t("storageColumnItem", "Item")}</span>
				<span className="text-right">{t("storageColumnSize", "Size")}</span>
				<span className="text-right max-md:hidden">
					{t("storageColumnChanged", "Last changed")}
				</span>
				<span />
			</div>
			{entries.map((entry) => {
				const { title, detail, badge } = describe(entry);
				return (
					<div
						key={entry.id}
						className="grid grid-cols-[28px_minmax(0,1fr)_80px_36px] items-center gap-3 px-4 py-3.5 transition-colors hover:bg-muted/30 md:grid-cols-[32px_minmax(0,1fr)_100px_150px_40px]"
					>
						<Checkbox
							aria-label={t("storageSelectItem", "Select {{name}}", {
								name: title,
							})}
							checked={selected.has(entry.id)}
							disabled={!entry.deletable}
							onCheckedChange={(checked) =>
								toggle([entry.id], checked === true)
							}
						/>
						<div className="min-w-0">
							<div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
								<p className="max-w-full truncate text-sm font-medium">
									{title}
								</p>
								{badge && (
									<Badge variant="outline" className="shrink-0">
										{badge}
									</Badge>
								)}
							</div>
							<p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
								{detail}
							</p>
						</div>
						<p className="text-right text-sm font-medium tabular-nums">
							{humanFileSize(entry.sizeBytes)}
						</p>
						<p className="text-right text-xs text-muted-foreground max-md:hidden">
							{entry.updatedAtMs ? formatStorageDate(entry.updatedAtMs) : "—"}
						</p>
						<Button
							variant="ghost"
							size="icon"
							disabled={!entry.deletable}
							aria-label={t("storageDeleteItem", "Delete {{name}}", {
								name: title,
							})}
							onClick={() => onDelete(entry)}
							className="size-8 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
						>
							<Trash2 className="size-4" />
						</Button>
					</div>
				);
			})}
		</div>
	);
}

function DeleteDialog({
	pending,
	onClose,
	onDeleted,
}: Readonly<{
	pending: PendingDelete | null;
	onClose: () => void;
	onDeleted: () => void;
}>) {
	const { t } = useTranslation("settings");
	const copy = useStorageCopy();
	const [deleting, setDeleting] = useState(false);

	const confirm = async () => {
		if (!pending) return;
		setDeleting(true);
		try {
			const result = await deleteBrowserStorageEntries(pending.entries, {
				clearBoardHistory,
			});
			if (result.deleted > 0) {
				toast.success(
					t("storageDeletedItems", {
						count: result.deleted,
						defaultValue_one: "Deleted {{count}} item",
						defaultValue_other: "Deleted {{count}} items",
					}),
				);
			}
			if (result.failed.length > 0) {
				toast.error(
					t("storageItemsNotDeleted", {
						count: result.failed.length,
						defaultValue_one: "{{count}} item could not be deleted",
						defaultValue_other: "{{count}} items could not be deleted",
					}),
					{ description: String(result.failed[0].error) },
				);
			}
			onClose();
			onDeleted();
		} finally {
			setDeleting(false);
		}
	};

	return (
		<AlertDialog
			open={pending !== null}
			onOpenChange={(open) => !open && !deleting && onClose()}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("storageDeleteTitle", {
							count: pending?.entries.length ?? 0,
							defaultValue_one: "Delete {{count}} item?",
							defaultValue_other: "Delete {{count}} items?",
						})}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"storageDeleteDescription",
							"This permanently removes about {{size}} from this browser.",
							{ size: humanFileSize(sumBytes(pending?.entries ?? [])) },
						)}{" "}
						{pending ? copy.deleteWarnings[pending.category] : null}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel disabled={deleting}>
						{t("storageCancel", "Cancel")}
					</AlertDialogCancel>
					<AlertDialogAction
						disabled={deleting}
						onClick={(event) => {
							event.preventDefault();
							void confirm();
						}}
						className="bg-destructive text-white hover:bg-destructive/90"
					>
						{deleting
							? t("storageDeleting", "Deleting…")
							: t("storageDeletePermanently", "Delete permanently")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function PersistenceCard({
	persisted,
	onChanged,
}: Readonly<{ persisted?: boolean; onChanged: () => void }>) {
	const { t } = useTranslation("settings");
	const [requesting, setRequesting] = useState(false);
	const supported = !!globalThis.navigator?.storage?.persist;
	if (persisted === undefined || !supported) return null;

	const request = async () => {
		setRequesting(true);
		try {
			const granted = await navigator.storage.persist();
			if (granted) {
				toast.success(
					t("storagePersistGranted", "This browser will keep Flow-Like's data"),
				);
				onChanged();
			} else {
				toast.warning(
					t(
						"storagePersistDenied",
						"The browser did not allow it right now. Installing Flow-Like as an app or bookmarking it usually helps.",
					),
				);
			}
		} finally {
			setRequesting(false);
		}
	};

	const Icon = persisted ? ShieldCheck : ShieldAlert;
	return (
		<Card>
			<CardHeader>
				<div className="flex gap-3">
					<div
						className={cn(
							"flex size-9 shrink-0 items-center justify-center rounded-lg",
							persisted
								? "bg-primary/10 text-primary"
								: "bg-destructive/10 text-destructive",
						)}
					>
						<Icon className="size-4.5" />
					</div>
					<div>
						<CardTitle className="text-base">
							{persisted
								? t("storagePersisted", "Kept when space runs low")
								: t(
										"storageNotPersisted",
										"May be removed when space runs low",
									)}
						</CardTitle>
						<CardDescription className="mt-1">
							{persisted
								? t(
										"storagePersistedDescription",
										"The browser keeps this data even when the disk fills up.",
									)
								: t(
										"storageNotPersistedDescription",
										"When the disk fills up, the browser may delete this data, including undo history and changes waiting to sync.",
									)}
						</CardDescription>
					</div>
				</div>
			</CardHeader>
			{!persisted && (
				<CardContent>
					<Button
						variant="secondary"
						size="sm"
						disabled={requesting}
						onClick={() => void request()}
					>
						{t("storagePersistRequest", "Ask the browser to keep it")}
					</Button>
				</CardContent>
			)}
		</Card>
	);
}

function LocalOnlyNote() {
	const { t } = useTranslation("settings");
	return (
		<div className="rounded-xl border border-dashed bg-muted/20 p-4">
			<div className="flex gap-3">
				<Info className="mt-0.5 size-4 shrink-0 text-primary" />
				<div className="space-y-1">
					<p className="text-sm font-medium">
						{t("storageOnlyThisBrowser", "Only this browser")}
					</p>
					<p className="text-xs leading-relaxed text-muted-foreground">
						{t(
							"storageOnlyThisBrowserDescription",
							"Everything here lives in this browser on this device. Deleting it never touches your apps, boards or files on the server. Sizes are estimates before the browser compresses them on disk.",
						)}
					</p>
				</div>
			</div>
		</div>
	);
}

function ScanPending({
	failed,
	progress,
	onRetry,
}: Readonly<{
	failed: boolean;
	progress?: ScanProgress;
	onRetry: () => void;
}>) {
	if (failed) return <ScanFailed onRetry={onRetry} />;
	return <StorageLoading progress={progress} />;
}

function StorageLoading({ progress }: Readonly<{ progress?: ScanProgress }>) {
	const { t } = useTranslation("settings");
	return (
		<div className="space-y-5">
			<Skeleton className="h-52 w-full rounded-2xl" />
			<p className="flex items-center gap-2 text-sm text-muted-foreground">
				<RefreshCw className="size-4 animate-spin" />
				{progress && progress.total > 1
					? t(
							"storageMeasuringProgress",
							"Measuring storage… {{done}} of {{total}}",
							{ done: progress.done, total: progress.total },
						)
					: t("storageMeasuring", "Measuring storage…")}
			</p>
			<div className="grid grid-cols-2 gap-3 md:grid-cols-3 2xl:grid-cols-6">
				{BROWSER_STORAGE_CATEGORIES.map((category) => (
					<Skeleton key={category} className="h-28 rounded-xl" />
				))}
			</div>
			<Skeleton className="h-80 w-full rounded-xl" />
		</div>
	);
}

function ScanFailed({ onRetry }: Readonly<{ onRetry: () => void }>) {
	const { t } = useTranslation("settings");
	return (
		<Card className="mx-auto max-w-md text-center">
			<CardHeader>
				<div className="mx-auto mb-2 flex size-11 items-center justify-center rounded-full bg-destructive/10 text-destructive">
					<TriangleAlert className="size-5" />
				</div>
				<CardTitle>
					{t("storageScanFailed", "Storage couldn’t be measured")}
				</CardTitle>
				<CardDescription>
					{t(
						"storageScanFailedDescription",
						"This browser did not let Flow-Like read its local storage. Private windows often block it.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<Button onClick={onRetry}>{t("storageTryAgain", "Try again")}</Button>
			</CardContent>
		</Card>
	);
}

function describeBoard(
	board: NonNullable<BrowserStorageEntry["board"]>,
	names: BoardNames,
	t: SettingsT,
): EntryDescription {
	const { appId, boardId, undoSteps, snapshotIds } = board;
	const appLabel =
		names.appName(appId) ??
		(appId ? t("storageUnknownApp", "App {{id}}", { id: shortId(appId) }) : "");
	const parts = [
		appLabel,
		undoSteps > 0
			? t("storageUndoSteps", {
					count: undoSteps,
					defaultValue_one: "{{count}} undo step",
					defaultValue_other: "{{count}} undo steps",
				})
			: "",
		snapshotIds.length > 0 ? t("storageChangeSnapshot", "change snapshot") : "",
	];
	const unavailable = names.appsLoaded && !!appId && !names.hasApp(appId);
	return {
		title:
			names.boardName(appId, boardId) ??
			t("storageUnknownBoard", "Board {{id}}", { id: shortId(boardId) }),
		detail: parts.filter(Boolean).join(" · "),
		badge: unavailable
			? t("storageAppUnavailable", "App unavailable")
			: undefined,
	};
}

function describeEntry(
	entry: BrowserStorageEntry,
	copy: StorageCopy,
	names: BoardNames,
	t: SettingsT,
): EntryDescription {
	if (entry.board) return describeBoard(entry.board, names, t);
	const badge = entry.deletable ? undefined : t("storageManaged", "Managed");
	if (entry.store) {
		const { label, description } = copy.stores[entry.store];
		return { title: label, detail: description, badge };
	}
	if (entry.unattributed) {
		return {
			title: t("storageUnattributed", "Browser overhead"),
			detail: t(
				"storageUnattributedDescription",
				"Indexes, metadata and space the browser has not reclaimed yet.",
			),
			badge,
		};
	}
	return {
		title: entry.database ?? entry.id,
		detail: t(
			"storageUnknownDatabase",
			"Database of a library or an older version of Flow-Like.",
		),
		badge,
	};
}

function shortId(id: string) {
	return id.length > 10 ? `${id.slice(0, 8)}…` : id;
}
