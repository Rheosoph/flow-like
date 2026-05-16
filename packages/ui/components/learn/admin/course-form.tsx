"use client";
import { ImageIcon, Loader2, Upload } from "lucide-react";
import {
	type ChangeEvent,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import type {
	CourseCategory,
	CourseDetail,
	CourseDifficulty,
} from "../../../lib/learn/types";
import { Button } from "../../ui/button";
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
import { Textarea } from "../../ui/textarea";

const ACCEPTED_IMAGE_TYPES = "image/jpeg,image/jpg,image/png,image/webp";
const VALID_IMAGE_TYPES = [
	"image/jpeg",
	"image/jpg",
	"image/png",
	"image/webp",
];
const MAX_ICON_SIZE = 20 * 1024 * 1024;
const MAX_THUMBNAIL_SIZE = 30 * 1024 * 1024;

const difficulties: ReadonlyArray<CourseDifficulty> = [
	"BEGINNER",
	"INTERMEDIATE",
	"ADVANCED",
	"EXPERT",
];

const categories: ReadonlyArray<CourseCategory> = [
	"GENERAL",
	"GETTING_STARTED",
	"FLOWS",
	"PAGES",
	"EVENTS",
	"DATA",
	"AI",
	"INTEGRATIONS",
	"DEPLOYMENT",
	"ADVANCED",
	"EXPERT",
];

export interface CourseFormValue {
	readonly language: string;
	readonly slug: string | null;
	readonly difficulty: CourseDifficulty;
	readonly category: CourseCategory;
	readonly estimated_minutes: number;
	readonly is_published: boolean;
	readonly tags: ReadonlyArray<string>;
	readonly position: number | null;
	readonly name: string;
	readonly description: string | null;
	readonly long_description: string | null;
}

interface CourseFormProps {
	readonly initial?: CourseDetail | null;
	readonly onSubmit: (value: CourseFormValue) => Promise<void> | void;
	readonly submitting?: boolean;
	readonly submitLabel?: string;
	readonly onUploadMedia?: (
		item: "icon" | "thumbnail",
		file: File,
	) => Promise<void> | void;
	readonly mediaUploading?: "icon" | "thumbnail" | null;
}

function fromDetail(d: CourseDetail | null | undefined): CourseFormValue {
	return {
		language: d?.language ?? "en",
		slug: d?.slug ?? null,
		difficulty: (d?.difficulty as CourseDifficulty) ?? "BEGINNER",
		category: (d?.category as CourseCategory) ?? "GENERAL",
		estimated_minutes: d?.estimated_minutes ?? 0,
		is_published: d?.is_published ?? false,
		tags: d?.tags ?? [],
		position: d?.position ?? null,
		name: d?.name ?? "",
		description: d?.description ?? null,
		long_description: d?.long_description ?? null,
	};
}

export function CourseForm({
	initial,
	onSubmit,
	submitting,
	submitLabel = "Save course",
	onUploadMedia,
	mediaUploading,
}: CourseFormProps) {
	const [value, setValue] = useState<CourseFormValue>(fromDetail(initial));
	const [mediaPreview, setMediaPreview] = useState<{
		readonly icon: string | null;
		readonly thumbnail: string | null;
	}>({ icon: null, thumbnail: null });
	const iconInputRef = useRef<HTMLInputElement>(null);
	const thumbnailInputRef = useRef<HTMLInputElement>(null);

	useEffect(() => {
		setValue(fromDetail(initial));
		setMediaPreview({ icon: null, thumbnail: null });
	}, [initial]);

	function patch<K extends keyof CourseFormValue>(
		key: K,
		v: CourseFormValue[K],
	) {
		setValue((prev) => ({ ...prev, [key]: v }));
	}

	const tagsString = value.tags.join(", ");
	const iconPreview = mediaPreview.icon ?? initial?.icon_url ?? null;
	const thumbnailPreview =
		mediaPreview.thumbnail ?? initial?.banner_url ?? null;
	const handleFileSelect = useCallback(
		async (item: "icon" | "thumbnail", e: ChangeEvent<HTMLInputElement>) => {
			const file = e.target.files?.[0];
			e.target.value = "";
			if (!file || !onUploadMedia) return;
			if (!VALID_IMAGE_TYPES.includes(file.type)) {
				toast.error("Use a PNG, JPEG, or WebP image.");
				return;
			}
			const maxSize = item === "icon" ? MAX_ICON_SIZE : MAX_THUMBNAIL_SIZE;
			if (file.size > maxSize) {
				toast.error(
					`File too large. Maximum size is ${Math.round(maxSize / 1024 / 1024)}MB.`,
				);
				return;
			}

			await onUploadMedia(item, file);
			const objectUrl = URL.createObjectURL(file);
			setMediaPreview((prev) => ({ ...prev, [item]: objectUrl }));
		},
		[onUploadMedia],
	);

	return (
		<form
			className="space-y-5"
			onSubmit={(e) => {
				e.preventDefault();
				void onSubmit(value);
			}}
		>
			<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
				<div className="space-y-2">
					<Label htmlFor="course-name">Title</Label>
					<Input
						id="course-name"
						value={value.name}
						onChange={(e) => patch("name", e.target.value)}
						placeholder="Build your first chat agent"
						required
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="course-slug">Slug</Label>
					<Input
						id="course-slug"
						value={value.slug ?? ""}
						onChange={(e) => patch("slug", e.target.value || null)}
						placeholder="first-chat-agent"
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="course-language">Language</Label>
					<Input
						id="course-language"
						value={value.language}
						onChange={(e) => patch("language", e.target.value)}
						placeholder="en"
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor="course-minutes">Estimated minutes</Label>
					<Input
						id="course-minutes"
						type="number"
						value={value.estimated_minutes}
						onChange={(e) =>
							patch("estimated_minutes", Number(e.target.value) || 0)
						}
					/>
				</div>
				<div className="space-y-2">
					<Label>Difficulty</Label>
					<Select
						value={value.difficulty}
						onValueChange={(v) => patch("difficulty", v as CourseDifficulty)}
					>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{difficulties.map((d) => (
								<SelectItem key={d} value={d}>
									{d}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<div className="space-y-2">
					<Label>Category</Label>
					<Select
						value={value.category}
						onValueChange={(v) => patch("category", v as CourseCategory)}
					>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{categories.map((c) => (
								<SelectItem key={c} value={c}>
									{c.replaceAll("_", " ")}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<div className="space-y-2">
					<Label htmlFor="course-position">Catalog position</Label>
					<Input
						id="course-position"
						type="number"
						value={value.position ?? ""}
						onChange={(e) => {
							const raw = e.target.value;
							if (raw === "") {
								patch("position", null);
								return;
							}
							const parsed = Number.parseInt(raw, 10);
							patch("position", Number.isFinite(parsed) ? parsed : null);
						}}
						placeholder="Leave empty to sort by name only"
					/>
					<p className="text-xs text-muted-foreground">
						Lower numbers appear first. Courses without a position fall to the
						end and sort alphabetically by title.
					</p>
				</div>
			</div>
			<div className="space-y-3">
				<div>
					<Label>Visual assets</Label>
					<p className="text-xs text-muted-foreground">
						Course icon and banner are stored on the course metadata and served
						through signed download URLs.
					</p>
				</div>
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
				<div className="grid grid-cols-1 gap-4 md:grid-cols-2">
					<div className="rounded-lg border border-border/70 bg-card/50 p-3">
						<div className="flex items-center gap-4">
							{iconPreview ? (
								<img
									src={iconPreview}
									alt="Course icon"
									className="h-16 w-16 rounded-xl border object-cover"
								/>
							) : (
								<div className="grid h-16 w-16 place-items-center rounded-xl border border-dashed bg-muted/40">
									<ImageIcon className="h-6 w-6 text-muted-foreground" />
								</div>
							)}
							<div className="min-w-0 flex-1 space-y-2">
								<div>
									<p className="text-sm font-medium">Icon</p>
									<p className="text-xs text-muted-foreground">
										Square image, max 20 MB.
									</p>
								</div>
								<Button
									type="button"
									variant="outline"
									size="sm"
									className="gap-1.5"
									disabled={!onUploadMedia || Boolean(mediaUploading)}
									onClick={() => iconInputRef.current?.click()}
								>
									{mediaUploading === "icon" ? (
										<Loader2 className="h-4 w-4 animate-spin" />
									) : (
										<Upload className="h-4 w-4" />
									)}
									{iconPreview ? "Change icon" : "Upload icon"}
								</Button>
							</div>
						</div>
					</div>
					<div className="rounded-lg border border-border/70 bg-card/50 p-3">
						<div className="flex items-center gap-4">
							{thumbnailPreview ? (
								<img
									src={thumbnailPreview}
									alt="Course banner"
									className="h-16 w-28 rounded-xl border object-cover"
								/>
							) : (
								<div className="grid h-16 w-28 place-items-center rounded-xl border border-dashed bg-muted/40">
									<ImageIcon className="h-6 w-6 text-muted-foreground" />
								</div>
							)}
							<div className="min-w-0 flex-1 space-y-2">
								<div>
									<p className="text-sm font-medium">Banner / thumbnail</p>
									<p className="text-xs text-muted-foreground">
										Wide image, max 30 MB.
									</p>
								</div>
								<Button
									type="button"
									variant="outline"
									size="sm"
									className="gap-1.5"
									disabled={!onUploadMedia || Boolean(mediaUploading)}
									onClick={() => thumbnailInputRef.current?.click()}
								>
									{mediaUploading === "thumbnail" ? (
										<Loader2 className="h-4 w-4 animate-spin" />
									) : (
										<Upload className="h-4 w-4" />
									)}
									{thumbnailPreview ? "Change banner" : "Upload banner"}
								</Button>
							</div>
						</div>
					</div>
				</div>
			</div>
			<div className="space-y-2">
				<Label htmlFor="course-tags">Tags (comma separated)</Label>
				<Input
					id="course-tags"
					value={tagsString}
					onChange={(e) =>
						patch(
							"tags",
							e.target.value
								.split(",")
								.map((t) => t.trim())
								.filter(Boolean),
						)
					}
				/>
			</div>
			<div className="space-y-2">
				<Label htmlFor="course-description">Short description</Label>
				<Textarea
					id="course-description"
					value={value.description ?? ""}
					onChange={(e) => patch("description", e.target.value || null)}
					rows={2}
					placeholder="One sentence summary shown on the catalog card."
				/>
			</div>
			<div className="space-y-2">
				<Label htmlFor="course-long-description">About this course</Label>
				<Textarea
					id="course-long-description"
					value={value.long_description ?? ""}
					onChange={(e) => patch("long_description", e.target.value || null)}
					rows={6}
					placeholder="Longer description shown on the course landing page."
				/>
			</div>
			<div className="flex items-center justify-between rounded-md border p-3">
				<div>
					<p className="text-sm font-medium">
						{value.is_published ? "Public" : "Draft"}
					</p>
					<p className="text-xs text-muted-foreground">
						Public courses are visible to everyone. Drafts are visible to users
						with ReadCourses, WriteCourses, or Admin.
					</p>
				</div>
				<Switch
					checked={value.is_published}
					onCheckedChange={(v) => patch("is_published", v)}
				/>
			</div>
			<Button type="submit" disabled={submitting}>
				{submitLabel}
			</Button>
		</form>
	);
}
