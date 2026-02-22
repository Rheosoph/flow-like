"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	BookOpen,
	ExternalLink,
	Globe,
	HelpCircle,
	ImageIcon,
	Loader2,
	Save,
	Tag,
	Trash2,
	Upload,
	X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import type {
	PackageMeta,
	PushMediaResponse,
	UpsertPackageMetaRequest,
} from "../../lib/schema/wasm";
import { useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	Input,
	Label,
	Separator,
	Textarea,
} from "../ui";
import type { GenericFetcher } from "../pages/store/store-package-detail";

const ACCEPTED_IMAGE_TYPES = "image/jpeg,image/png,image/webp";
const MAX_ICON_SIZE = 20 * 1024 * 1024;
const MAX_THUMBNAIL_SIZE = 30 * 1024 * 1024;

export interface PackageMetaTabProps {
	packageId: string;
	fetcher: GenericFetcher;
	auth?: unknown;
}

export function PackageMetaTab({
	packageId,
	fetcher,
	auth,
}: PackageMetaTabProps) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const queryClient = useQueryClient();
	const iconInputRef = useRef<HTMLInputElement>(null);
	const thumbnailInputRef = useRef<HTMLInputElement>(null);

	const queryKey = ["package-meta", packageId];

	const { data: meta, isLoading } = useQuery<PackageMeta | null>({
		queryKey,
		queryFn: async () => {
			try {
				return await fetcher<PackageMeta>(
					profile.data!.hub_profile,
					`registry/package/${packageId}/meta`,
					{ method: "GET" },
					auth,
				);
			} catch {
				return null;
			}
		},
		enabled: !!profile.data,
	});

	const [form, setForm] = useState<UpsertPackageMetaRequest>({
		name: "",
	});
	const [newTag, setNewTag] = useState("");
	const [hasChanges, setHasChanges] = useState(false);

	useEffect(() => {
		if (meta) {
			setForm({
				name: meta.name,
				description: meta.description,
				longDescription: meta.longDescription,
				tags: meta.tags ?? [],
				website: meta.website,
				supportUrl: meta.supportUrl,
				docsUrl: meta.docsUrl,
				useCase: meta.useCase,
				releaseNotes: meta.releaseNotes,
				ageRating: meta.ageRating,
			});
		}
	}, [meta]);

	useEffect(() => {
		if (!meta) {
			setHasChanges(form.name.trim().length > 0);
			return;
		}
		const changed =
			form.name !== meta.name ||
			(form.description ?? "") !== (meta.description ?? "") ||
			(form.longDescription ?? "") !== (meta.longDescription ?? "") ||
			(form.website ?? "") !== (meta.website ?? "") ||
			(form.supportUrl ?? "") !== (meta.supportUrl ?? "") ||
			(form.docsUrl ?? "") !== (meta.docsUrl ?? "") ||
			(form.useCase ?? "") !== (meta.useCase ?? "") ||
			(form.releaseNotes ?? "") !== (meta.releaseNotes ?? "") ||
			JSON.stringify(form.tags ?? []) !== JSON.stringify(meta.tags ?? []);
		setHasChanges(changed);
	}, [form, meta]);

	const update = useCallback(
		(patch: Partial<UpsertPackageMetaRequest>) =>
			setForm((prev) => ({ ...prev, ...patch })),
		[],
	);

	const addTag = useCallback(
		(tag: string) => {
			const trimmed = tag.trim();
			if (!trimmed || form.tags?.includes(trimmed)) return;
			update({ tags: [...(form.tags ?? []), trimmed] });
			setNewTag("");
		},
		[form.tags, update],
	);

	const removeTag = useCallback(
		(tagToRemove: string) => {
			update({ tags: (form.tags ?? []).filter((t) => t !== tagToRemove) });
		},
		[form.tags, update],
	);

	const handleTagKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter") {
				e.preventDefault();
				addTag(newTag);
			}
		},
		[newTag, addTag],
	);

	const saveMutation = useMutation({
		mutationFn: () =>
			fetcher<PackageMeta>(
				profile.data!.hub_profile,
				`registry/package/${packageId}/meta`,
				{
					method: "PUT",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify(form),
				},
				auth,
			),
		onSuccess: () => {
			toast.success("Metadata saved");
			queryClient.invalidateQueries({ queryKey });
			queryClient.invalidateQueries({
				queryKey: ["registry-package", packageId],
			});
		},
		onError: () => toast.error("Failed to save metadata"),
	});

	const invalidateMeta = useCallback(() => {
		queryClient.invalidateQueries({ queryKey });
		queryClient.invalidateQueries({
			queryKey: ["registry-package", packageId],
		});
	}, [queryClient, queryKey, packageId]);

	const uploadMedia = useCallback(
		async (item: "icon" | "thumbnail", file: File) => {
			if (!profile.data) return;
			const ext = file.name.split(".").pop() ?? "png";
			const { signed_url } = await fetcher<PushMediaResponse>(
				profile.data.hub_profile,
				`registry/package/${packageId}/meta/media?language=en&item=${item}&extension=${ext}`,
				{ method: "PUT" },
				auth,
			);
			await fetch(signed_url, {
				method: "PUT",
				body: file,
				headers: { "Content-Type": file.type },
			});
			invalidateMeta();
		},
		[profile.data, fetcher, packageId, auth, invalidateMeta],
	);

	const iconUpload = useMutation({
		mutationFn: (file: File) => uploadMedia("icon", file),
		onSuccess: () => toast.success("Icon uploaded"),
		onError: () => toast.error("Failed to upload icon"),
	});

	const thumbnailUpload = useMutation({
		mutationFn: (file: File) => uploadMedia("thumbnail", file),
		onSuccess: () => toast.success("Thumbnail uploaded"),
		onError: () => toast.error("Failed to upload thumbnail"),
	});

	const handleFileSelect = useCallback(
		(item: "icon" | "thumbnail", e: React.ChangeEvent<HTMLInputElement>) => {
			const file = e.target.files?.[0];
			if (!file) return;
			const maxSize =
				item === "icon" ? MAX_ICON_SIZE : MAX_THUMBNAIL_SIZE;
			if (file.size > maxSize) {
				toast.error(
					`File too large. Maximum size is ${Math.round(maxSize / 1024 / 1024)}MB`,
				);
				return;
			}
			if (item === "icon") iconUpload.mutate(file);
			else thumbnailUpload.mutate(file);
			e.target.value = "";
		},
		[iconUpload, thumbnailUpload],
	);

	const isUploading = iconUpload.isPending || thumbnailUpload.isPending;

	if (isLoading) {
		return (
			<Card>
				<CardContent className="flex items-center justify-center py-12">
					<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
				</CardContent>
			</Card>
		);
	}

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<div>
					<h3 className="text-lg font-semibold">Package Metadata</h3>
					<p className="text-sm text-muted-foreground">
						Customize how your package appears in the store
					</p>
				</div>
				<Button
					onClick={() => saveMutation.mutate()}
					disabled={!hasChanges || saveMutation.isPending}
					className="gap-2"
				>
					{saveMutation.isPending ? (
						<Loader2 className="h-4 w-4 animate-spin" />
					) : (
						<Save className="h-4 w-4" />
					)}
					Save Changes
				</Button>
			</div>

			{/* Basic Info */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base">Basic Information</CardTitle>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="meta-name">Display Name</Label>
						<Input
							id="meta-name"
							value={form.name}
							onChange={(e) => update({ name: e.target.value })}
							placeholder="Package display name"
						/>
					</div>
					<div className="space-y-2">
						<Label htmlFor="meta-description">Short Description</Label>
						<Input
							id="meta-description"
							value={form.description ?? ""}
							onChange={(e) => update({ description: e.target.value })}
							placeholder="Brief description of your package"
						/>
					</div>
					<div className="space-y-2">
						<Label htmlFor="meta-long-description">Long Description</Label>
						<Textarea
							id="meta-long-description"
							value={form.longDescription ?? ""}
							onChange={(e) => update({ longDescription: e.target.value })}
							placeholder="Detailed description, features, usage instructions..."
							rows={8}
						/>
					</div>
					<div className="space-y-2">
						<Label htmlFor="meta-use-case">Use Case</Label>
						<Input
							id="meta-use-case"
							value={form.useCase ?? ""}
							onChange={(e) => update({ useCase: e.target.value })}
							placeholder="What problem does this package solve?"
						/>
					</div>
				</CardContent>
			</Card>

			{/* Tags */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base flex items-center gap-2">
						<Tag className="h-4 w-4" />
						Tags
					</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3">
					<Input
						value={newTag}
						onChange={(e) => setNewTag(e.target.value)}
						onKeyDown={handleTagKeyDown}
						placeholder="Type a tag and press Enter..."
					/>
					{(form.tags?.length ?? 0) > 0 && (
						<div className="flex flex-wrap gap-1.5">
							{form.tags?.map((tag) => (
								<Badge
									key={tag}
									variant="secondary"
									className="gap-1 pr-1"
								>
									{tag}
									<button
										type="button"
										onClick={() => removeTag(tag)}
										className="ml-0.5 rounded-full hover:bg-muted-foreground/20 p-0.5"
									>
										<X className="h-3 w-3" />
									</button>
								</Badge>
							))}
						</div>
					)}
				</CardContent>
			</Card>

			{/* Branding */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base flex items-center gap-2">
						<ImageIcon className="h-4 w-4" />
						Branding
					</CardTitle>
				</CardHeader>
				<CardContent className="space-y-4">
					<input
						ref={iconInputRef}
						type="file"
						accept={ACCEPTED_IMAGE_TYPES}
						className="hidden"
						onChange={(e) => handleFileSelect("icon", e)}
					/>
					<input
						ref={thumbnailInputRef}
						type="file"
						accept={ACCEPTED_IMAGE_TYPES}
						className="hidden"
						onChange={(e) => handleFileSelect("thumbnail", e)}
					/>
					<div className="grid grid-cols-1 md:grid-cols-2 gap-6">
						<div className="space-y-3">
							<Label>Icon</Label>
							<div className="flex items-center gap-4">
								{meta?.icon ? (
									<img
										src={meta.icon}
										alt="Icon"
										className="h-16 w-16 rounded-lg object-cover border"
									/>
								) : (
									<div className="h-16 w-16 rounded-lg border border-dashed flex items-center justify-center bg-muted/50">
										<ImageIcon className="h-6 w-6 text-muted-foreground" />
									</div>
								)}
								<Button
									variant="outline"
									size="sm"
									className="gap-1.5"
									disabled={isUploading}
									onClick={() => iconInputRef.current?.click()}
								>
									{iconUpload.isPending ? (
										<Loader2 className="h-4 w-4 animate-spin" />
									) : (
										<Upload className="h-4 w-4" />
									)}
									Upload Icon
								</Button>
							</div>
							<p className="text-xs text-muted-foreground">
								Square image, max 20 MB. PNG, JPEG, or WebP.
							</p>
						</div>
						<div className="space-y-3">
							<Label>Thumbnail</Label>
							<div className="flex items-center gap-4">
								{meta?.thumbnail ? (
									<img
										src={meta.thumbnail}
										alt="Thumbnail"
										className="h-16 w-28 rounded-lg object-cover border"
									/>
								) : (
									<div className="h-16 w-28 rounded-lg border border-dashed flex items-center justify-center bg-muted/50">
										<ImageIcon className="h-6 w-6 text-muted-foreground" />
									</div>
								)}
								<Button
									variant="outline"
									size="sm"
									className="gap-1.5"
									disabled={isUploading}
									onClick={() => thumbnailInputRef.current?.click()}
								>
									{thumbnailUpload.isPending ? (
										<Loader2 className="h-4 w-4 animate-spin" />
									) : (
										<Upload className="h-4 w-4" />
									)}
									Upload Thumbnail
								</Button>
							</div>
							<p className="text-xs text-muted-foreground">
								16:9 or similar, max 30 MB. PNG, JPEG, or WebP.
							</p>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Links */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base flex items-center gap-2">
						<Globe className="h-4 w-4" />
						Links
					</CardTitle>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="meta-website" className="flex items-center gap-1.5">
							<ExternalLink className="h-3.5 w-3.5" />
							Website
						</Label>
						<Input
							id="meta-website"
							type="url"
							value={form.website ?? ""}
							onChange={(e) => update({ website: e.target.value })}
							placeholder="https://example.com"
						/>
					</div>
					<div className="space-y-2">
						<Label htmlFor="meta-docs" className="flex items-center gap-1.5">
							<BookOpen className="h-3.5 w-3.5" />
							Documentation
						</Label>
						<Input
							id="meta-docs"
							type="url"
							value={form.docsUrl ?? ""}
							onChange={(e) => update({ docsUrl: e.target.value })}
							placeholder="https://docs.example.com"
						/>
					</div>
					<div className="space-y-2">
						<Label htmlFor="meta-support" className="flex items-center gap-1.5">
							<HelpCircle className="h-3.5 w-3.5" />
							Support
						</Label>
						<Input
							id="meta-support"
							type="url"
							value={form.supportUrl ?? ""}
							onChange={(e) => update({ supportUrl: e.target.value })}
							placeholder="https://support.example.com"
						/>
					</div>
				</CardContent>
			</Card>

			{/* Release Notes */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base">Release Notes</CardTitle>
				</CardHeader>
				<CardContent>
					<Textarea
						value={form.releaseNotes ?? ""}
						onChange={(e) => update({ releaseNotes: e.target.value })}
						placeholder="What's new in this version..."
						rows={4}
					/>
				</CardContent>
			</Card>
		</div>
	);
}
