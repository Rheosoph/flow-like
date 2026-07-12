"use client";

import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Checkbox,
	Input,
	Skeleton,
	Switch,
	cn,
} from "@flow-like/flow-like-ui";
import { humanFileSize } from "@flow-like/flow-like-ui/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import {
	Archive,
	ArrowLeft,
	Box,
	CheckCircle2,
	Clock3,
	Database,
	FileClock,
	Files,
	FolderKanban,
	HardDrive,
	Info,
	PanelsTopLeft,
	RefreshCw,
	Search,
	ShieldCheck,
	Sparkles,
	Trash2,
	TriangleAlert,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import {
	useInvalidateTauriInvoke,
	useTauriInvoke,
} from "../../../components/useInvoke";

interface StorageItem {
	id: string;
	name: string;
	detail: string;
	sizeBytes: number;
	fileCount: number;
	updatedAtMs?: number;
	deletable: boolean;
}

interface StorageCategory {
	key:
		| "apps"
		| "bits"
		| "logs"
		| "offloaded"
		| "browser"
		| "cache"
		| "temporary";
	label: string;
	description: string;
	sizeBytes: number;
	itemCount: number;
	items: StorageItem[];
}

interface LogRetentionPolicy {
	enabled: boolean;
	days: number;
	lastCleanupMs?: number;
}

interface StorageOverview {
	totalBytes: number;
	generatedAtMs: number;
	logRetention: LogRetentionPolicy;
	categories: StorageCategory[];
}

interface DeleteResult {
	deletedItems: number;
	freedBytes: number;
	skippedItems: string[];
}

const CATEGORY_META = {
	apps: {
		icon: FolderKanban,
		color: "bg-sky-500",
		soft: "bg-sky-500/10 text-sky-600 dark:text-sky-400",
	},
	bits: {
		icon: Box,
		color: "bg-violet-500",
		soft: "bg-violet-500/10 text-violet-600 dark:text-violet-400",
	},
	logs: {
		icon: FileClock,
		color: "bg-amber-500",
		soft: "bg-amber-500/10 text-amber-600 dark:text-amber-400",
	},
	offloaded: {
		icon: Files,
		color: "bg-fuchsia-500",
		soft: "bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-400",
	},
	browser: {
		icon: PanelsTopLeft,
		color: "bg-cyan-500",
		soft: "bg-cyan-500/10 text-cyan-600 dark:text-cyan-400",
	},
	cache: {
		icon: Database,
		color: "bg-emerald-500",
		soft: "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
	},
	temporary: {
		icon: Archive,
		color: "bg-rose-500",
		soft: "bg-rose-500/10 text-rose-600 dark:text-rose-400",
	},
} as const;

const RETENTION_OPTIONS = [7, 14, 30, 60, 90, 180, 365];

function estimateValueBytes(
	value: unknown,
	seen = new WeakSet<object>(),
): number {
	if (value === null || value === undefined) return 0;
	if (typeof value === "string") return value.length * 2;
	if (typeof value === "number" || typeof value === "bigint") return 8;
	if (typeof value === "boolean") return 4;
	if (value instanceof Blob) return value.size;
	if (value instanceof ArrayBuffer) return value.byteLength;
	if (ArrayBuffer.isView(value)) return value.byteLength;
	if (value instanceof Date) return 8;
	if (typeof value !== "object" || seen.has(value)) return 0;
	seen.add(value);
	if (Array.isArray(value)) {
		return value.reduce(
			(total, entry) => total + estimateValueBytes(entry, seen),
			0,
		);
	}
	return Object.entries(value as Record<string, unknown>).reduce(
		(total, [key, entry]) =>
			total + key.length * 2 + estimateValueBytes(entry, seen),
		0,
	);
}

function scanObjectStore(store: IDBObjectStore) {
	return new Promise<{ bytes: number; records: number }>((resolve, reject) => {
		let bytes = 0;
		let records = 0;
		const request = store.openCursor();
		request.onsuccess = () => {
			const cursor = request.result;
			if (!cursor) {
				resolve({ bytes, records });
				return;
			}
			records += 1;
			bytes +=
				estimateValueBytes(cursor.key) + estimateValueBytes(cursor.value);
			cursor.continue();
		};
		request.onerror = () => reject(request.error);
	});
}

async function scanIndexedDatabase(name: string, version?: number) {
	const database = await new Promise<IDBDatabase>((resolve, reject) => {
		const request = indexedDB.open(name, version);
		request.onsuccess = () => resolve(request.result);
		request.onerror = () => reject(request.error);
		request.onupgradeneeded = () => {
			request.transaction?.abort();
			reject(
				new Error(`IndexedDB ${name} changed while it was being measured`),
			);
		};
	});
	try {
		const stores = Array.from(database.objectStoreNames);
		if (stores.length === 0) return { bytes: 0, records: 0, stores: 0 };
		const transaction = database.transaction(stores, "readonly");
		const results = await Promise.all(
			stores.map((store) => scanObjectStore(transaction.objectStore(store))),
		);
		return {
			bytes: results.reduce((total, result) => total + result.bytes, 0),
			records: results.reduce((total, result) => total + result.records, 0),
			stores: stores.length,
		};
	} finally {
		database.close();
	}
}

async function inspectBrowserStorage(): Promise<StorageCategory> {
	const items: StorageItem[] = [];
	let measuredBytes = 0;
	try {
		for (let index = 0; index < localStorage.length; index += 1) {
			const key = localStorage.key(index);
			if (!key) continue;
			const value = localStorage.getItem(key) ?? "";
			const sizeBytes = (key.length + value.length) * 2;
			measuredBytes += sizeBytes;
			items.push({
				id: `browser:local:${key}`,
				name: key,
				detail: "Local preference stored by the Studio WebView",
				sizeBytes,
				fileCount: 1,
				deletable: false,
			});
		}
	} catch {
		// localStorage can be unavailable in hardened WebViews.
	}

	const factory = indexedDB as IDBFactory & {
		databases?: () => Promise<Array<{ name?: string; version?: number }>>;
	};
	if (factory.databases) {
		const databases = await factory.databases().catch(() => []);
		for (const info of databases) {
			if (!info.name) continue;
			try {
				const measured = await scanIndexedDatabase(info.name, info.version);
				measuredBytes += measured.bytes;
				items.push({
					id: `browser:idb:${info.name}`,
					name: info.name,
					detail: `IndexedDB · ${measured.stores} ${measured.stores === 1 ? "store" : "stores"} · ${measured.records.toLocaleString()} records`,
					sizeBytes: measured.bytes,
					fileCount: measured.records,
					deletable: false,
				});
			} catch {
				items.push({
					id: `browser:idb:${info.name}`,
					name: info.name,
					detail: "IndexedDB · currently in use",
					sizeBytes: 0,
					fileCount: 0,
					deletable: false,
				});
			}
		}
	}

	const originUsage = await navigator.storage
		?.estimate()
		.then((estimate) => estimate.usage ?? 0)
		.catch(() => 0);
	const otherBytes = Math.max(0, originUsage - measuredBytes);
	if (otherBytes > 0) {
		items.push({
			id: "browser:other",
			name: "Other WebView data",
			detail: "Browser-managed storage not attributable to one database",
			sizeBytes: otherBytes,
			fileCount: 0,
			deletable: false,
		});
	}
	items.sort((a, b) => b.sizeBytes - a.sizeBytes);
	return {
		key: "browser",
		label: "Browser storage",
		description:
			"Local preferences and IndexedDB records kept inside Studio's embedded WebView. Sizes are estimates.",
		sizeBytes: Math.max(measuredBytes, originUsage),
		itemCount: items.length,
		items,
	};
}

function formatDate(timestamp?: number) {
	if (!timestamp) return "Unknown";
	return new Intl.DateTimeFormat(undefined, {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(new Date(timestamp));
}

function StorageLoading() {
	return (
		<div className="space-y-5">
			<Skeleton className="h-44 w-full rounded-2xl" />
			<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
				{[
					"apps",
					"bits",
					"logs",
					"offloaded",
					"browser",
					"cache",
					"temporary",
				].map((key) => (
					<Skeleton key={key} className="h-28 rounded-xl" />
				))}
			</div>
			<Skeleton className="h-80 w-full rounded-xl" />
		</div>
	);
}

export default function LocalStoragePage() {
	const overviewQuery = useTauriInvoke<StorageOverview>(
		"get_local_storage_overview",
		{},
	);
	const invalidate = useInvalidateTauriInvoke();
	const [activeCategory, setActiveCategory] =
		useState<StorageCategory["key"]>("logs");
	const [selected, setSelected] = useState<Set<string>>(new Set());
	const [search, setSearch] = useState("");
	const [pendingDelete, setPendingDelete] = useState<StorageItem[] | null>(
		null,
	);
	const [retentionEnabled, setRetentionEnabled] = useState<boolean | null>(
		null,
	);
	const [retentionDays, setRetentionDays] = useState<number | null>(null);
	const [savingPolicy, setSavingPolicy] = useState(false);
	const [deleting, setDeleting] = useState(false);
	const [browserCategory, setBrowserCategory] =
		useState<StorageCategory | null>(null);
	const [browserScanNonce, setBrowserScanNonce] = useState(0);

	useEffect(() => {
		const scanGeneration = browserScanNonce;
		let cancelled = false;
		void inspectBrowserStorage().then((result) => {
			if (!cancelled && scanGeneration === browserScanNonce) {
				setBrowserCategory(result);
			}
		});
		return () => {
			cancelled = true;
		};
	}, [browserScanNonce]);

	const overview = overviewQuery.data;
	const categories = useMemo(() => {
		const nativeCategories = [...(overview?.categories ?? [])];
		if (browserCategory) {
			const cacheIndex = nativeCategories.findIndex(
				(entry) => entry.key === "cache",
			);
			nativeCategories.splice(
				cacheIndex < 0 ? nativeCategories.length : cacheIndex,
				0,
				browserCategory,
			);
		}
		return nativeCategories;
	}, [browserCategory, overview?.categories]);
	const totalBytes = categories.reduce(
		(total, entry) => total + entry.sizeBytes,
		0,
	);
	const policy = overview?.logRetention;
	const enabled = retentionEnabled ?? policy?.enabled ?? false;
	const days = retentionDays ?? policy?.days ?? 30;
	const category = categories.find((entry) => entry.key === activeCategory);
	const filteredItems = useMemo(() => {
		const query = search.trim().toLocaleLowerCase();
		if (!query) return category?.items ?? [];
		return (category?.items ?? []).filter(
			(item) =>
				item.name.toLocaleLowerCase().includes(query) ||
				item.detail.toLocaleLowerCase().includes(query) ||
				item.id.toLocaleLowerCase().includes(query),
		);
	}, [category, search]);
	const selectedItems = (category?.items ?? []).filter((item) =>
		selected.has(item.id),
	);
	const selectedBytes = selectedItems.reduce(
		(total, item) => total + item.sizeBytes,
		0,
	);

	const refresh = async () => {
		setBrowserCategory(null);
		setBrowserScanNonce((current) => current + 1);
		await invalidate("get_local_storage_overview");
	};

	const changeCategory = (key: StorageCategory["key"]) => {
		setActiveCategory(key);
		setSelected(new Set());
		setSearch("");
	};

	const savePolicy = async (nextEnabled = enabled, nextDays = days) => {
		setSavingPolicy(true);
		try {
			const result = await invoke<DeleteResult>("set_log_retention_policy", {
				policy: { enabled: nextEnabled, days: nextDays },
			});
			setRetentionEnabled(nextEnabled);
			setRetentionDays(nextDays);
			await refresh();
			if (result.deletedItems > 0) {
				toast.success(
					`Removed ${result.deletedItems} old ${result.deletedItems === 1 ? "run" : "runs"}`,
					{ description: `${humanFileSize(result.freedBytes)} freed` },
				);
			} else {
				toast.success(
					nextEnabled
						? "Automatic cleanup enabled"
						: "Automatic cleanup disabled",
				);
			}
		} catch (error) {
			toast.error("Could not update log retention", {
				description: String(error),
			});
		} finally {
			setSavingPolicy(false);
		}
	};

	const deleteItems = async () => {
		if (!pendingDelete?.length || !category) return;
		setDeleting(true);
		try {
			const result = await invoke<DeleteResult>("delete_local_storage_items", {
				category: category.key,
				ids: pendingDelete.map((item) => item.id),
			});
			toast.success(
				`Deleted ${result.deletedItems} ${result.deletedItems === 1 ? "item" : "items"}`,
				{
					description: `${humanFileSize(result.freedBytes)} freed from this device`,
				},
			);
			if (result.skippedItems.length) {
				toast.warning(
					`${result.skippedItems.length} item(s) could not be deleted`,
				);
			}
			setSelected(new Set());
			setPendingDelete(null);
			await refresh();
		} catch (error) {
			toast.error("Could not delete the selected files", {
				description: String(error),
			});
		} finally {
			setDeleting(false);
		}
	};

	if (overviewQuery.isLoading) {
		return (
			<div className="h-full overflow-auto px-1 pb-6">
				<div className="mx-auto max-w-screen-2xl space-y-6">
					<PageHeader refreshing={false} onRefresh={() => {}} />
					<StorageLoading />
				</div>
			</div>
		);
	}

	if (overviewQuery.isError || !overview) {
		return (
			<div className="flex h-full items-center justify-center">
				<Card className="max-w-md text-center">
					<CardHeader>
						<div className="mx-auto mb-2 flex size-11 items-center justify-center rounded-full bg-destructive/10 text-destructive">
							<TriangleAlert className="size-5" />
						</div>
						<CardTitle>Storage couldn’t be inspected</CardTitle>
						<CardDescription>
							Studio could not read one or more local storage directories.
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Button onClick={refresh}>Try again</Button>
					</CardContent>
				</Card>
			</div>
		);
	}

	return (
		<div className="h-full overflow-auto px-1 pb-8">
			<div className="mx-auto max-w-screen-2xl space-y-5">
				<PageHeader refreshing={overviewQuery.isFetching} onRefresh={refresh} />

				<section className="relative overflow-hidden rounded-2xl border bg-gradient-to-br from-card via-card to-primary/[0.07] p-5 shadow-sm md:p-7">
					<div className="pointer-events-none absolute -right-24 -top-28 size-80 rounded-full bg-primary/10 blur-3xl" />
					<div className="relative grid items-center gap-8 lg:grid-cols-[minmax(0,1fr)_auto]">
						<div className="space-y-5">
							<div className="flex items-start gap-3">
								<div className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-sm">
									<HardDrive className="size-5" />
								</div>
								<div>
									<p className="text-sm font-medium text-muted-foreground">
										Used by Flow-Like Studio
									</p>
									<p className="text-4xl font-semibold tracking-tight md:text-5xl">
										{humanFileSize(totalBytes)}
									</p>
								</div>
							</div>
							<div className="grid gap-x-7 gap-y-3 sm:grid-cols-2 lg:grid-cols-3">
								{categories.map((entry) => {
									const meta = CATEGORY_META[entry.key];
									return (
										<button
											type="button"
											key={entry.key}
											onClick={() => changeCategory(entry.key)}
											className="group flex min-w-0 items-center gap-2 text-left"
										>
											<span
												className={cn("size-2.5 rounded-full", meta.color)}
											/>
											<span className="truncate text-sm text-muted-foreground group-hover:text-foreground">
												{entry.label}
											</span>
											<span className="ml-auto text-sm font-semibold tabular-nums">
												{humanFileSize(entry.sizeBytes)}
											</span>
										</button>
									);
								})}
							</div>
						</div>
						<div className="hidden size-36 rounded-full border-[14px] border-primary/10 p-3 lg:flex lg:items-center lg:justify-center">
							<div className="flex size-full flex-col items-center justify-center rounded-full bg-background/80 text-center shadow-inner">
								<span className="text-lg font-semibold">
									{categories.reduce((sum, item) => sum + item.itemCount, 0)}
								</span>
								<span className="text-xs text-muted-foreground">
									stored items
								</span>
							</div>
						</div>
					</div>
				</section>

				<div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_360px]">
					<div className="min-w-0 space-y-5">
						<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4 2xl:grid-cols-7">
							{categories.map((entry) => {
								const meta = CATEGORY_META[entry.key];
								const Icon = meta.icon;
								return (
									<button
										type="button"
										key={entry.key}
										onClick={() => changeCategory(entry.key)}
										className={cn(
											"rounded-xl border bg-card p-4 text-left shadow-xs transition-all hover:-translate-y-0.5 hover:shadow-md",
											activeCategory === entry.key &&
												"border-primary ring-2 ring-primary/15",
										)}
									>
										<div
											className={cn(
												"mb-4 flex size-9 items-center justify-center rounded-lg",
												meta.soft,
											)}
										>
											<Icon className="size-4.5" />
										</div>
										<p className="truncate text-sm font-medium">
											{entry.label}
										</p>
										<div className="mt-1 flex items-baseline justify-between gap-2">
											<p className="text-lg font-semibold tabular-nums">
												{humanFileSize(entry.sizeBytes)}
											</p>
											<p className="text-xs text-muted-foreground">
												{entry.itemCount} items
											</p>
										</div>
									</button>
								);
							})}
						</div>

						<Card className="overflow-hidden">
							<CardHeader className="border-b bg-muted/20 pb-4">
								<div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
									<div>
										<div className="flex items-center gap-2">
											<CardTitle>{category?.label}</CardTitle>
											<Badge variant="secondary">
												{category?.itemCount ?? 0}
											</Badge>
										</div>
										<CardDescription className="mt-1 max-w-2xl">
											{category?.description}
										</CardDescription>
									</div>
									{selectedItems.length > 0 && (
										<Button
											variant="destructive"
											size="sm"
											onClick={() => setPendingDelete(selectedItems)}
										>
											<Trash2 /> Delete {selectedItems.length} ·{" "}
											{humanFileSize(selectedBytes)}
										</Button>
									)}
								</div>
								<div className="relative mt-1">
									<Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
									<Input
										value={search}
										onChange={(event) => setSearch(event.target.value)}
										placeholder={`Search ${category?.label.toLocaleLowerCase() ?? "files"}`}
										className="pl-9"
									/>
								</div>
							</CardHeader>
							<CardContent className="p-0">
								{filteredItems.length === 0 ? (
									<div className="flex min-h-56 flex-col items-center justify-center px-6 text-center">
										<div className="mb-3 flex size-11 items-center justify-center rounded-full bg-muted text-muted-foreground">
											<CheckCircle2 className="size-5" />
										</div>
										<p className="font-medium">
											{search ? "No matching items" : "Nothing stored here"}
										</p>
										<p className="mt-1 text-sm text-muted-foreground">
											{search
												? "Try a different search."
												: "This category is already clean."}
										</p>
									</div>
								) : (
									<div className="divide-y">
										<div className="grid grid-cols-[32px_minmax(0,1fr)_100px_138px_40px] items-center gap-3 px-4 py-2.5 text-xs font-medium uppercase tracking-wide text-muted-foreground max-md:grid-cols-[32px_minmax(0,1fr)_88px_40px]">
											<Checkbox
												aria-label="Select all visible items"
												checked={
													filteredItems.filter((item) => item.deletable)
														.length > 0 &&
													filteredItems
														.filter((item) => item.deletable)
														.every((item) => selected.has(item.id))
												}
												onCheckedChange={(checked) => {
													const next = new Set(selected);
													for (const item of filteredItems.filter(
														(entry) => entry.deletable,
													)) {
														if (checked) next.add(item.id);
														else next.delete(item.id);
													}
													setSelected(next);
												}}
											/>
											<span>Item</span>
											<span className="text-right">Size</span>
											<span className="text-right max-md:hidden">
												Last changed
											</span>
											<span />
										</div>
										{filteredItems.map((item) => (
											<div
												key={item.id}
												className="grid grid-cols-[32px_minmax(0,1fr)_100px_138px_40px] items-center gap-3 px-4 py-3.5 transition-colors hover:bg-muted/30 max-md:grid-cols-[32px_minmax(0,1fr)_88px_40px]"
											>
												<Checkbox
													aria-label={`Select ${item.name}`}
													checked={selected.has(item.id)}
													disabled={!item.deletable}
													onCheckedChange={(checked) => {
														const next = new Set(selected);
														if (checked) next.add(item.id);
														else next.delete(item.id);
														setSelected(next);
													}}
												/>
												<div className="min-w-0">
													<div className="flex items-center gap-2">
														<p className="truncate text-sm font-medium">
															{item.name}
														</p>
														{!item.deletable && (
															<Badge variant="outline">
																{activeCategory === "browser" ||
																activeCategory === "offloaded" ||
																activeCategory === "cache"
																	? "Managed"
																	: "In use"}
															</Badge>
														)}
													</div>
													<p className="mt-0.5 truncate text-xs text-muted-foreground">
														{item.detail}
													</p>
												</div>
												<p className="text-right text-sm font-medium tabular-nums">
													{humanFileSize(item.sizeBytes)}
												</p>
												<p className="text-right text-xs text-muted-foreground max-md:hidden">
													{formatDate(item.updatedAtMs)}
												</p>
												<Button
													variant="ghost"
													size="icon"
													disabled={!item.deletable}
													aria-label={`Delete ${item.name}`}
													onClick={() => setPendingDelete([item])}
													className="size-8 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
												>
													<Trash2 className="size-4" />
												</Button>
											</div>
										))}
									</div>
								)}
							</CardContent>
						</Card>
					</div>

					<aside className="space-y-5">
						<Card className="overflow-hidden border-amber-500/25">
							<div className="h-1 bg-gradient-to-r from-amber-400 via-orange-400 to-rose-400" />
							<CardHeader>
								<div className="flex items-start justify-between gap-4">
									<div className="flex gap-3">
										<div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-amber-500/10 text-amber-600 dark:text-amber-400">
											<Sparkles className="size-4.5" />
										</div>
										<div>
											<CardTitle className="text-base">
												Automatic log cleanup
											</CardTitle>
											<CardDescription className="mt-1">
												Keep debug runs from growing unnoticed.
											</CardDescription>
										</div>
									</div>
									<Switch
										checked={enabled}
										disabled={savingPolicy}
										onCheckedChange={(checked) => {
											setRetentionEnabled(checked);
											void savePolicy(checked, days);
										}}
										aria-label="Automatically delete old run logs"
									/>
								</div>
							</CardHeader>
							<CardContent className="space-y-4">
								<div
									className={cn(
										"space-y-2 transition-opacity",
										!enabled && "opacity-50",
									)}
								>
									<label
										htmlFor="retention-days"
										className="text-sm font-medium"
									>
										Delete run logs older than
									</label>
									<div className="grid grid-cols-4 gap-2">
										{RETENTION_OPTIONS.slice(0, 4).map((option) => (
											<Button
												key={option}
												type="button"
												variant={days === option ? "default" : "outline"}
												size="sm"
												disabled={!enabled || savingPolicy}
												onClick={() => {
													setRetentionDays(option);
													void savePolicy(enabled, option);
												}}
											>
												{option}d
											</Button>
										))}
									</div>
									<div className="flex items-center gap-2">
										<Input
											id="retention-days"
											type="number"
											min={1}
											max={3650}
											value={days}
											disabled={!enabled || savingPolicy}
											onChange={(event) =>
												setRetentionDays(
													Math.max(
														1,
														Math.min(3650, Number(event.target.value)),
													),
												)
											}
										/>
										<Button
											variant="secondary"
											disabled={
												!enabled || savingPolicy || days === policy?.days
											}
											onClick={() => savePolicy(enabled, days)}
										>
											Save
										</Button>
									</div>
								</div>
								<div className="flex gap-2.5 rounded-lg bg-muted/60 p-3 text-xs text-muted-foreground">
									<ShieldCheck className="mt-0.5 size-4 shrink-0 text-foreground" />
									<p>
										Only completed local run logs are removed. Apps, boards, and
										active runs are never part of automatic cleanup.
									</p>
								</div>
								{policy?.lastCleanupMs && (
									<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
										<Clock3 className="size-3.5" /> Last checked{" "}
										{formatDate(policy.lastCleanupMs)}
									</p>
								)}
							</CardContent>
						</Card>

						<div className="rounded-xl border border-dashed bg-muted/20 p-4">
							<div className="flex gap-3">
								<Info className="mt-0.5 size-4 shrink-0 text-primary" />
								<div className="space-y-1">
									<p className="text-sm font-medium">
										Everything shown here is local
									</p>
									<p className="text-xs leading-relaxed text-muted-foreground">
										This page inspects files on this device only. It does not
										count or delete cloud data.
									</p>
								</div>
							</div>
						</div>
					</aside>
				</div>
			</div>

			<AlertDialog
				open={pendingDelete !== null}
				onOpenChange={(open) => !open && setPendingDelete(null)}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							Delete {pendingDelete?.length ?? 0} local{" "}
							{pendingDelete?.length === 1 ? "item" : "items"}?
						</AlertDialogTitle>
						<AlertDialogDescription>
							This will permanently remove{" "}
							{humanFileSize(
								pendingDelete?.reduce((sum, item) => sum + item.sizeBytes, 0) ??
									0,
							)}{" "}
							from this device.
							{activeCategory === "apps" &&
								" Deleted apps and their local project files cannot be recovered."}
							{activeCategory === "bits" &&
								" Downloaded bits can be downloaded again when needed."}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel disabled={deleting}>Cancel</AlertDialogCancel>
						<AlertDialogAction
							disabled={deleting}
							onClick={(event) => {
								event.preventDefault();
								void deleteItems();
							}}
							className="bg-destructive text-white hover:bg-destructive/90"
						>
							{deleting ? "Deleting…" : "Delete permanently"}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}

function PageHeader({
	refreshing,
	onRefresh,
}: {
	refreshing: boolean;
	onRefresh: () => void | Promise<void>;
}) {
	return (
		<div className="flex items-start justify-between gap-4 pt-1">
			<div>
				<Link
					href="/settings"
					className="mb-2 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
				>
					<ArrowLeft className="size-4" /> Settings
				</Link>
				<h1 className="text-3xl font-bold tracking-tight">Local storage</h1>
				<p className="mt-1 text-muted-foreground">
					Understand what Studio keeps on this device and take control of it.
				</p>
			</div>
			<Button
				variant="outline"
				size="sm"
				onClick={onRefresh}
				disabled={refreshing}
			>
				<RefreshCw className={cn(refreshing && "animate-spin")} /> Refresh
			</Button>
		</div>
	);
}
