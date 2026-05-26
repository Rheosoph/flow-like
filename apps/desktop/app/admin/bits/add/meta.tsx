import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type IBit,
	type IMetadata,
	Input,
	Label,
	Slider,
	Textarea,
	humanFileSize,
} from "@flow-like/flow-like-ui";
import { X } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";

export function MetaConfiguration({
	bit,
	setBit,
}: Readonly<{ bit: IBit; setBit: Dispatch<SetStateAction<IBit>> }>) {
	const getMeta = (field: keyof IMetadata) => {
		return bit.meta?.["en"]?.[field];
	};

	const updateMeta = <K extends keyof IMetadata>(
		field: K,
		value: IMetadata[K],
	) => {
		setBit((old) => ({
			...old,
			meta: {
				...old.meta,
				en: {
					...old.meta?.["en"],
					[field]: value,
				},
			},
		}));
	};

	const addTag = (tag: string) => {
		if (!tag.trim()) return;
		const currentTags = bit.meta?.["en"]?.tags || [];
		if (!currentTags.includes(tag.trim())) {
			updateMeta("tags", [...currentTags, tag.trim()]);
		}
	};

	const removeTag = (tagToRemove: string) => {
		const currentTags = bit.meta?.["en"]?.tags || [];
		updateMeta(
			"tags",
			currentTags.filter((tag) => tag !== tagToRemove),
		);
	};

	const addPreviewMedia = (url: string) => {
		if (!url.trim()) return;
		const currentMedia = bit.meta?.["en"]?.preview_media || [];
		if (!currentMedia.includes(url.trim())) {
			updateMeta("preview_media", [...currentMedia, url.trim()]);
		}
	};

	const removePreviewMedia = (urlToRemove: string) => {
		const currentMedia = bit.meta?.["en"]?.preview_media || [];
		updateMeta(
			"preview_media",
			currentMedia.filter((url) => url !== urlToRemove),
		);
	};

	const addAuthor = (author: string) => {
		if (!author.trim()) return;
		const currentAuthors = bit.authors || [];
		if (!currentAuthors.includes(author.trim())) {
			setBit((old) => ({
				...old,
				authors: [...currentAuthors, author.trim()],
			}));
		}
	};

	const removeAuthor = (authorToRemove: string) => {
		const currentAuthors = bit.authors || [];
		setBit((old) => ({
			...old,
			authors: currentAuthors.filter((author) => author !== authorToRemove),
		}));
	};

	return (
		<div className="space-y-6 w-full max-w-screen-lg">
			<Card className="w-full">
				<CardHeader>
					<CardTitle className="flex items-center justify-between">
						Metadata Configuration
						<small className="font-normal text-muted-foreground">
							{humanFileSize(bit.size ?? 0)}
						</small>
					</CardTitle>
					<CardDescription>
						Configure basic information about the model
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
						<div className="space-y-2">
							<Label htmlFor="name">Name *</Label>
							<Input
								id="name"
								value={getMeta("name") || ""}
								onChange={(e) => updateMeta("name", e.target.value)}
								placeholder="Model name"
								required
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="filename">Filename</Label>
							<Input
								id="filename"
								value={bit.file_name || ""}
								onChange={(e) =>
									setBit((old) => ({ ...old, file_name: e.target.value }))
								}
								placeholder="model-file.bin"
								required
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="age-rating">
								Age Rating [{getMeta("age_rating") ?? 0}+]
							</Label>
							<Slider
								id="age-rating"
								min={0}
								max={18}
								step={1}
								value={[Number.parseInt(getMeta("age_rating") || "") || 0]}
								onValueChange={(value) => updateMeta("age_rating", value[0])}
								className="py-3.5"
							/>
						</div>
					</div>

					<div className="space-y-2">
						<Label htmlFor="description">Description *</Label>
						<Textarea
							id="description"
							value={getMeta("description") || ""}
							onChange={(e) => updateMeta("description", e.target.value)}
							placeholder="Brief description of the model"
							rows={2}
							required
						/>
					</div>

					<div className="space-y-2">
						<Label htmlFor="long-description">Long Description</Label>
						<Textarea
							id="long-description"
							value={getMeta("long_description") || ""}
							onChange={(e) => updateMeta("long_description", e.target.value)}
							placeholder="Detailed description of the model's capabilities and use cases"
							rows={4}
						/>
					</div>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>URLs & Links</CardTitle>
					<CardDescription>
						Configure relevant URLs for the model
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
						<div className="space-y-2">
							<Label htmlFor="website">Website URL</Label>
							<Input
								id="website"
								type="url"
								value={getMeta("website") || ""}
								onChange={(e) => updateMeta("website", e.target.value)}
								placeholder="https://example.com"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="docs-url">Documentation URL</Label>
							<Input
								id="docs-url"
								type="url"
								value={getMeta("docs_url") || ""}
								onChange={(e) => updateMeta("docs_url", e.target.value)}
								placeholder="https://docs.example.com"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="support-url">Support URL</Label>
							<Input
								id="support-url"
								type="url"
								value={getMeta("support_url") || ""}
								onChange={(e) => updateMeta("support_url", e.target.value)}
								placeholder="https://support.example.com"
							/>
						</div>
					</div>

					<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
						<div className="space-y-2">
							<Label htmlFor="repo-url">Repository URL</Label>
							<Input
								id="repo-url"
								type="url"
								value={bit.repository || ""}
								onChange={(e) =>
									setBit((old) => ({ ...old, repository: e.target.value }))
								}
								placeholder="https://huggingface.co/deepseek-ai/DeepSeek-R1-0528-Qwen3-8B"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="use-case">Use Case</Label>
							<Input
								id="use-case"
								value={getMeta("use_case") || ""}
								onChange={(e) => updateMeta("use_case", e.target.value)}
								placeholder="e.g., Chat, Code Generation, Analysis"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="license">License</Label>
							<Input
								id="license"
								value={bit.license || ""}
								onChange={(e) =>
									setBit((old) => ({ ...old, license: e.target.value }))
								}
								placeholder="e.g., MIT, Apache-2.0, CC-BY-NC-4.0"
							/>
						</div>
					</div>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>Media & Assets</CardTitle>
					<CardDescription>
						Configure icons, thumbnails and preview media
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
						<div className="space-y-2">
							<Label htmlFor="icon">Icon URL</Label>
							<Input
								id="icon"
								type="url"
								value={getMeta("icon") || ""}
								onChange={(e) => updateMeta("icon", e.target.value)}
								placeholder="https://example.com/icon.png"
							/>
						</div>

						<div className="space-y-2">
							<Label htmlFor="thumbnail">Thumbnail URL</Label>
							<Input
								id="thumbnail"
								type="url"
								value={getMeta("thumbnail") || ""}
								onChange={(e) => updateMeta("thumbnail", e.target.value)}
								placeholder="https://example.com/thumbnail.png"
							/>
						</div>
					</div>

					<div className="space-y-2">
						<Label htmlFor="preview-media">Preview Media URLs</Label>
						<div className="space-y-2">
							{(getMeta("preview_media") || []).map(
								(url: string, index: number) => (
									<div key={index} className="flex items-center gap-2">
										<Input
											value={url}
											onChange={(e) => {
												const newMedia = [...(getMeta("preview_media") || [])];
												newMedia[index] = e.target.value;
												updateMeta("preview_media", newMedia);
											}}
											placeholder="https://example.com/preview.png"
										/>
										<Button
											type="button"
											variant="destructive"
											size="sm"
											onClick={() => removePreviewMedia(url)}
										>
											Remove
										</Button>
									</div>
								),
							)}
							<Input
								placeholder="Add preview media URL and press Enter"
								onKeyDown={(e) => {
									if (e.key === "Enter") {
										e.preventDefault();
										addPreviewMedia(e.currentTarget.value);
										e.currentTarget.value = "";
									}
								}}
							/>
						</div>
					</div>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>Authors</CardTitle>
					<CardDescription>
						Add the authors or contributors of this model
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="add-author">Add Author</Label>
						<div className="flex gap-2">
							<Input
								id="add-author"
								placeholder="e.g., DeepSeek AI, OpenAI"
								onKeyDown={(e) => {
									if (e.key === "Enter") {
										e.preventDefault();
										addAuthor(e.currentTarget.value);
										e.currentTarget.value = "";
									}
								}}
							/>
							<Button
								type="button"
								onClick={() => {
									const input = document.getElementById(
										"add-author",
									) as HTMLInputElement;
									addAuthor(input.value);
									input.value = "";
								}}
							>
								Add
							</Button>
						</div>
					</div>
					<div className="flex flex-wrap gap-2">
						{(bit.authors || []).map((author: string, index: number) => (
							<Badge
								key={index}
								variant="secondary"
								className="flex items-center gap-1"
							>
								{author}
								<Button
									size="sm"
									variant="ghost"
									className="h-4 w-4 p-0"
									onClick={() => removeAuthor(author)}
								>
									<X className="h-3 w-3" />
								</Button>
							</Badge>
						))}
					</div>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>Tags</CardTitle>
					<CardDescription>
						Add relevant tags for categorization
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="add-tag">Add Tag</Label>
						<div className="flex gap-2">
							<Input
								id="add-tag"
								placeholder="e.g., llm, chat, coding"
								onKeyDown={(e) => {
									if (e.key === "Enter") {
										e.preventDefault();
										addTag(e.currentTarget.value);
										e.currentTarget.value = "";
									}
								}}
							/>
							<Button
								type="button"
								onClick={() => {
									const input = document.getElementById(
										"add-tag",
									) as HTMLInputElement;
									addTag(input.value);
									input.value = "";
								}}
							>
								Add
							</Button>
						</div>
					</div>
					<div className="flex flex-wrap gap-2">
						{(getMeta("tags") || []).map((tag: string, index: number) => (
							<Badge
								key={index}
								variant="secondary"
								className="flex items-center gap-1"
							>
								{tag}
								<Button
									size="sm"
									variant="ghost"
									className="h-4 w-4 p-0"
									onClick={() => removeTag(tag)}
								>
									<X className="h-3 w-3" />
								</Button>
							</Badge>
						))}
					</div>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>Release Notes</CardTitle>
					<CardDescription>
						Document what&apos;s new in this version
					</CardDescription>
				</CardHeader>
				<CardContent>
					<div className="space-y-2">
						<Label htmlFor="release-notes">Release Notes</Label>
						<Textarea
							id="release-notes"
							value={getMeta("release_notes") || ""}
							onChange={(e) => updateMeta("release_notes", e.target.value)}
							placeholder="What's new in this version?"
							rows={4}
						/>
					</div>
				</CardContent>
			</Card>
		</div>
	);
}
