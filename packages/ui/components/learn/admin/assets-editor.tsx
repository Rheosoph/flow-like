"use client";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Copy,
	FileAudio,
	FileText,
	Image as ImageIcon,
	Pencil,
	Plus,
	Sparkles,
	Trash2,
	Upload,
	Video,
} from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import type {
	CourseAsset,
	CourseAssetKind,
	CreateCourseAssetBody,
	OptimizeCourseAssetResponse,
} from "../../../lib/learn/types";
import type { IProfile } from "../../../types";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Input } from "../../ui/input";

export type AssetsEditorAuth = unknown;

/**
 * Minimal contract the assets section needs from a `learnApi` instance.
 * Both `apps/desktop/lib/learn-api.ts` and `apps/web/lib/learn-api.ts`
 * already satisfy this — keeping it narrow lets the section ship via
 * packages/ui without pulling in the rest of the API surface.
 */
export interface AssetsEditorApi {
	listCourseAssets(
		profile: IProfile,
		auth: AssetsEditorAuth,
		courseId: string,
		opts?: { kind?: CourseAssetKind },
	): Promise<CourseAsset[]>;
	uploadCourseAsset(
		profile: IProfile,
		auth: AssetsEditorAuth,
		courseId: string,
		body: CreateCourseAssetBody,
		file: File,
	): Promise<CourseAsset>;
	renameCourseAsset(
		profile: IProfile,
		auth: AssetsEditorAuth,
		courseId: string,
		assetId: string,
		name: string,
	): Promise<CourseAsset>;
	deleteCourseAsset(
		profile: IProfile,
		auth: AssetsEditorAuth,
		courseId: string,
		assetId: string,
	): Promise<void>;
	optimizeCourseAsset(
		profile: IProfile,
		auth: AssetsEditorAuth,
		courseId: string,
		assetId: string,
	): Promise<OptimizeCourseAssetResponse>;
}

export interface AssetsEditorProps {
	readonly api: AssetsEditorApi;
	readonly profile: IProfile | null;
	readonly auth: AssetsEditorAuth;
	readonly courseId: string;
}

const ASSET_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_-]{0,63}$/;

function inferKind(file: File): CourseAssetKind {
	const mime = file.type.toLowerCase();
	if (mime.startsWith("image/")) return "IMAGE";
	if (mime.startsWith("video/")) return "VIDEO";
	if (mime.startsWith("audio/")) return "AUDIO";
	return "DOCUMENT";
}

function suggestNameFromFile(file: File, taken: ReadonlySet<string>): string {
	const stem =
		file.name
			.replace(/\.[^.]+$/, "")
			.replace(/[^A-Za-z0-9_-]+/g, "_")
			.replace(/^_+|_+$/g, "") || "Asset";
	const base = /^[A-Za-z_]/.test(stem) ? stem : `Asset_${stem}`;
	if (!taken.has(base)) return base;
	let i = 2;
	while (taken.has(`${base}_${i}`)) i++;
	return `${base}_${i}`;
}

function formatBytes(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	if (bytes < 1024 * 1024 * 1024)
		return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function AssetKindIcon({ kind }: { readonly kind: CourseAssetKind }) {
	if (kind === "IMAGE") return <ImageIcon className="h-4 w-4" />;
	if (kind === "VIDEO") return <Video className="h-4 w-4" />;
	if (kind === "AUDIO") return <FileAudio className="h-4 w-4" />;
	return <FileText className="h-4 w-4" />;
}

export function AssetsEditor({
	api,
	profile,
	auth,
	courseId,
}: AssetsEditorProps) {
	const queryClient = useQueryClient();
	const profileId = profile?.id ?? "no-profile";
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for course authoring.");
		}
		return profile;
	};

	const fileInputRef = useRef<HTMLInputElement>(null);
	const [pendingFile, setPendingFile] = useState<File | null>(null);
	const [pendingName, setPendingName] = useState("");
	const [editingId, setEditingId] = useState<string | null>(null);
	const [editName, setEditName] = useState("");

	const assetsQuery = useQuery({
		queryKey: ["learn", "admin", "assets", courseId, profileId],
		enabled: Boolean(profile && courseId),
		queryFn: () => api.listCourseAssets(getProfile(), auth, courseId),
	});

	const assets = assetsQuery.data ?? [];
	const takenNames = useMemo(
		() => new Set(assets.map((a) => a.name)),
		[assets],
	);
	const optimizableAssets = useMemo(
		() =>
			assets.filter((a) => a.kind === "IMAGE" && a.mime_type !== "image/webp"),
		[assets],
	);
	const [bulkOptimize, setBulkOptimize] = useState<{
		readonly currentId: string | null;
		readonly done: number;
		readonly total: number;
	} | null>(null);

	const invalidate = () =>
		queryClient.invalidateQueries({
			queryKey: ["learn", "admin", "assets", courseId],
		});

	const upload = useMutation({
		mutationFn: async () => {
			if (!pendingFile) throw new Error("No file selected");
			const trimmed = pendingName.trim();
			if (!ASSET_NAME_PATTERN.test(trimmed)) {
				throw new Error(
					"Reference name must start with a letter or underscore and contain only letters, digits, '_' or '-' (max 64).",
				);
			}
			const extension =
				pendingFile.name.split(".").pop()?.toLowerCase() ?? "bin";
			return api.uploadCourseAsset(
				getProfile(),
				auth,
				courseId,
				{
					name: trimmed,
					filename: pendingFile.name,
					mime_type: pendingFile.type || "application/octet-stream",
					size: pendingFile.size,
					kind: inferKind(pendingFile),
					extension,
				},
				pendingFile,
			);
		},
		onSuccess: (asset) => {
			setPendingFile(null);
			setPendingName("");
			if (fileInputRef.current) fileInputRef.current.value = "";
			invalidate();
			toast.success(`Uploaded @${asset.name}`);
		},
		onError: (err: Error) => {
			toast.error(err.message || "Could not upload asset");
		},
	});

	const rename = useMutation({
		mutationFn: ({ id, name }: { id: string; name: string }) =>
			api.renameCourseAsset(getProfile(), auth, courseId, id, name),
		onSuccess: () => {
			setEditingId(null);
			setEditName("");
			invalidate();
			toast.success("Asset renamed");
		},
		onError: (err: Error) => {
			toast.error(err.message || "Could not rename asset");
		},
	});

	const remove = useMutation({
		mutationFn: (id: string) =>
			api.deleteCourseAsset(getProfile(), auth, courseId, id),
		onSuccess: () => {
			invalidate();
			toast.success("Asset deleted");
		},
	});

	const optimize = useMutation({
		mutationFn: (id: string) =>
			api.optimizeCourseAsset(getProfile(), auth, courseId, id),
		onSuccess: ({ asset, previous_size }) => {
			invalidate();
			const saved = previous_size - asset.size;
			const pct = previous_size > 0 ? (saved / previous_size) * 100 : 0;
			if (saved > 0) {
				toast.success(
					`Optimized @${asset.name}: ${formatBytes(previous_size)} → ${formatBytes(asset.size)} (-${pct.toFixed(0)}%)`,
				);
			} else {
				toast.success(`Optimized @${asset.name} (${formatBytes(asset.size)})`);
			}
		},
		onError: (err: Error) => {
			toast.error(err.message || "Could not optimize asset");
		},
	});

	const runOptimizeAll = async () => {
		if (optimizableAssets.length === 0 || bulkOptimize) return;
		const targets = [...optimizableAssets];
		setBulkOptimize({
			currentId: targets[0]?.id ?? null,
			done: 0,
			total: targets.length,
		});
		let succeeded = 0;
		let failed = 0;
		let savedTotal = 0;
		const profileSnap = getProfile();
		for (let i = 0; i < targets.length; i++) {
			const asset = targets[i];
			setBulkOptimize({
				currentId: asset.id,
				done: i,
				total: targets.length,
			});
			try {
				const res = await api.optimizeCourseAsset(
					profileSnap,
					auth,
					courseId,
					asset.id,
				);
				succeeded += 1;
				savedTotal += Math.max(0, res.previous_size - res.asset.size);
			} catch (err) {
				failed += 1;
				console.error(`Optimize failed for @${asset.name}`, err);
			}
		}
		setBulkOptimize(null);
		invalidate();
		if (failed === 0) {
			toast.success(
				`Optimized ${succeeded} image${succeeded === 1 ? "" : "s"} (saved ${formatBytes(savedTotal)})`,
			);
		} else if (succeeded === 0) {
			toast.error(`Could not optimize any images (${failed} failed)`);
		} else {
			toast.warning(
				`Optimized ${succeeded}, ${failed} failed (saved ${formatBytes(savedTotal)})`,
			);
		}
	};

	const copyReference = async (asset: CourseAsset) => {
		try {
			await navigator.clipboard.writeText(`@${asset.name}`);
			toast.success(`Copied @${asset.name}`);
		} catch {
			toast.error("Could not copy reference");
		}
	};

	return (
		<Card>
			<CardHeader>
				<div className="flex items-start justify-between gap-3">
					<div>
						<CardTitle className="text-base">Assets</CardTitle>
						<CardDescription>
							Upload images, videos, audio or documents and reference them in
							lesson content via{" "}
							<code className="rounded bg-muted px-1 py-0.5 text-xs">
								@AssetName
							</code>
							. Images render inline; other types render as links. Presigned
							URLs are generated on read.
						</CardDescription>
					</div>
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={runOptimizeAll}
						disabled={
							optimizableAssets.length === 0 ||
							bulkOptimize !== null ||
							!profile
						}
						title="Re-encode all non-WebP images as WebP"
					>
						<Sparkles className="h-3.5 w-3.5 mr-1.5" />
						{bulkOptimize
							? `Optimizing ${bulkOptimize.done + 1}/${bulkOptimize.total}…`
							: optimizableAssets.length > 0
								? `Optimize ${optimizableAssets.length} image${optimizableAssets.length === 1 ? "" : "s"}`
								: "All images optimized"}
					</Button>
				</div>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="space-y-3 rounded-md border p-3 bg-muted/20">
					<div className="flex items-center gap-2">
						<input
							ref={fileInputRef}
							type="file"
							className="hidden"
							onChange={(e) => {
								const file = e.target.files?.[0] ?? null;
								setPendingFile(file);
								if (file) {
									setPendingName(suggestNameFromFile(file, takenNames));
								}
							}}
						/>
						<Button
							type="button"
							variant="outline"
							onClick={() => fileInputRef.current?.click()}
							disabled={upload.isPending || !profile}
						>
							<Upload className="h-4 w-4 mr-2" />
							{pendingFile ? "Change file" : "Choose file"}
						</Button>
						{pendingFile ? (
							<span className="text-sm text-muted-foreground truncate">
								{pendingFile.name} · {formatBytes(pendingFile.size)}
							</span>
						) : (
							<span className="text-sm text-muted-foreground">
								No file selected.
							</span>
						)}
					</div>
					{pendingFile ? (
						<div className="flex items-end gap-2">
							<div className="flex-1 space-y-1.5">
								<label
									htmlFor="asset-name"
									className="text-xs font-medium text-muted-foreground"
								>
									Reference name
								</label>
								<Input
									id="asset-name"
									value={pendingName}
									onChange={(e) => setPendingName(e.target.value)}
									placeholder="Image1"
								/>
							</div>
							<Button
								type="button"
								onClick={() => upload.mutate()}
								disabled={upload.isPending || !pendingName.trim()}
							>
								<Plus className="h-4 w-4 mr-2" />
								{upload.isPending ? "Uploading…" : "Upload"}
							</Button>
						</div>
					) : null}
				</div>

				{assets.length === 0 ? (
					<p className="text-sm text-muted-foreground">No assets yet.</p>
				) : (
					<ul className="space-y-2">
						{assets.map((asset) => (
							<li
								key={asset.id}
								className="flex items-center gap-3 rounded-md border px-3 py-2"
							>
								<div className="text-muted-foreground">
									<AssetKindIcon kind={asset.kind} />
								</div>
								{editingId === asset.id ? (
									<form
										className="flex flex-1 items-center gap-2"
										onSubmit={(e) => {
											e.preventDefault();
											const trimmed = editName.trim();
											if (!ASSET_NAME_PATTERN.test(trimmed)) {
												toast.error(
													"Invalid reference name. Use letters, digits, '_' or '-'.",
												);
												return;
											}
											if (trimmed === asset.name) {
												setEditingId(null);
												return;
											}
											rename.mutate({ id: asset.id, name: trimmed });
										}}
									>
										<Input
											autoFocus
											value={editName}
											onChange={(e) => setEditName(e.target.value)}
											onKeyDown={(e) => {
												if (e.key === "Escape") {
													e.preventDefault();
													setEditingId(null);
												}
											}}
											className="h-8"
											disabled={rename.isPending}
										/>
										<Button
											type="submit"
											size="sm"
											disabled={rename.isPending || !editName.trim()}
										>
											Save
										</Button>
										<Button
											type="button"
											variant="ghost"
											size="sm"
											onClick={() => setEditingId(null)}
											disabled={rename.isPending}
										>
											Cancel
										</Button>
									</form>
								) : (
									<>
										<button
											type="button"
											className="text-left flex-1 truncate"
											onClick={() => copyReference(asset)}
											title="Click to copy reference"
										>
											<span className="font-mono text-sm">@{asset.name}</span>
											<span className="ml-2 text-xs text-muted-foreground truncate">
												{asset.filename} · {formatBytes(asset.size)}
											</span>
										</button>
										<Badge variant="secondary" className="text-[10px]">
											{asset.kind}
										</Badge>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => copyReference(asset)}
											title="Copy reference"
										>
											<Copy className="h-3 w-3" />
										</Button>
										{asset.kind === "IMAGE" &&
										asset.mime_type !== "image/webp" ? (
											<Button
												variant="ghost"
												size="sm"
												onClick={() => optimize.mutate(asset.id)}
												disabled={
													bulkOptimize !== null ||
													(optimize.isPending &&
														optimize.variables === asset.id)
												}
												title={
													bulkOptimize?.currentId === asset.id
														? "Optimizing…"
														: "Re-encode as WebP and replace original"
												}
											>
												<Sparkles
													className={`h-3 w-3 ${bulkOptimize?.currentId === asset.id ? "animate-pulse" : ""}`}
												/>
											</Button>
										) : null}
										<Button
											variant="outline"
											size="sm"
											onClick={() => {
												setEditingId(asset.id);
												setEditName(asset.name);
											}}
										>
											<Pencil className="h-3 w-3" />
										</Button>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => {
												if (
													confirm(
														`Delete @${asset.name}? References in lesson content will stop resolving.`,
													)
												) {
													remove.mutate(asset.id);
												}
											}}
										>
											<Trash2 className="h-3 w-3" />
										</Button>
									</>
								)}
							</li>
						))}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}
