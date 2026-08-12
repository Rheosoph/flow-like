import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type HuggingFaceGgufRepositoryImport,
	type HuggingFaceGgufSelectionOptions,
	type HuggingFaceModelImport,
	type IBit,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	humanFileSize,
	inferMlxAssetBitType,
	inspectHuggingFaceModelRepository,
	mlxAssetPathError,
	resolveHuggingFaceGgufSelection,
} from "@flow-like/flow-like-ui";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { CheckCircle2, Loader2, Plus, Search, Trash2 } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import { useRef, useState } from "react";
import { getModelSize } from "../utils";

type ModelImportHandler = (
	model: HuggingFaceModelImport,
	options?: HuggingFaceGgufSelectionOptions,
) => void;

export function HuggingFaceModelImporter({
	disabled,
	onImported,
}: Readonly<{
	disabled?: boolean;
	onImported: ModelImportHandler;
}>) {
	const [repository, setRepository] = useState("");
	const [inspecting, setInspecting] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [imported, setImported] = useState<HuggingFaceModelImport | null>(null);
	const [variantId, setVariantId] = useState<string | undefined>();
	const [projectorPath, setProjectorPath] = useState<string | undefined>();
	const [kind, setKind] = useState<"llm" | "vlm" | undefined>();
	const inspectionSequence = useRef(0);

	const applyGgufSelection = (
		model: HuggingFaceGgufRepositoryImport,
		options: HuggingFaceGgufSelectionOptions,
	): boolean => {
		try {
			resolveHuggingFaceGgufSelection(model, options);
			setError(null);
			onImported(model, options);
			return true;
		} catch (cause) {
			setError(
				cause instanceof Error
					? cause.message
					: "The selected GGUF files are not usable",
			);
			return false;
		}
	};

	const inspectRepository = async (reference = repository) => {
		const sequence = ++inspectionSequence.current;
		setInspecting(true);
		setError(null);
		try {
			const result = await inspectHuggingFaceModelRepository(
				reference,
				(input, init) => tauriFetch(input, init),
			);
			if (sequence !== inspectionSequence.current) return;
			if (result.access.private || result.access.gated !== false) {
				throw new Error(
					"Private and gated repositories cannot be mirrored into the shared store",
				);
			}
			setImported(result);
			if (result.format === "mlx") {
				setVariantId(undefined);
				setProjectorPath(undefined);
				setKind(result.kind);
				onImported(result);
				return;
			}

			const nextKind = result.kind === "unknown" ? undefined : result.kind;
			const requestedVariantId = result.variants.find(
				(variant) => variant.requested,
			)?.id;
			const nextOptions: HuggingFaceGgufSelectionOptions = {
				variantId: requestedVariantId ?? result.recommendedVariantId,
				projectorPath: result.recommendedProjectorPath,
				kind: nextKind,
			};
			setVariantId(nextOptions.variantId);
			setProjectorPath(nextOptions.projectorPath);
			setKind(nextKind);
			if (nextKind) applyGgufSelection(result, nextOptions);
		} catch (cause) {
			if (sequence !== inspectionSequence.current) return;
			setImported(null);
			setError(
				cause instanceof Error
					? cause.message
					: "Failed to inspect the Hugging Face repository",
			);
		} finally {
			if (sequence === inspectionSequence.current) setInspecting(false);
		}
	};

	const overrideKind = (nextKind: "llm" | "vlm") => {
		if (!imported) return;
		if (imported.format === "gguf") {
			if (
				applyGgufSelection(imported, {
					variantId,
					projectorPath,
					kind: nextKind,
				})
			) {
				setKind(nextKind);
			}
			return;
		}
		if (
			nextKind === "vlm" &&
			!imported.assets.some((asset) =>
				/^(processor_config|preprocessor_config)\.json$/i.test(asset.path),
			)
		) {
			setError(
				"This repository has no processor metadata required by an MLX VLM",
			);
			return;
		}
		setKind(nextKind);
		setError(null);
		if (imported.kind === nextKind) return;
		const overridden = {
			...imported,
			kind: nextKind,
			kindEvidence: [`Manually selected ${nextKind.toUpperCase()}`],
		};
		setImported(overridden);
		onImported(overridden);
	};

	const selectedVariant =
		imported?.format === "gguf"
			? imported.variants.find((variant) => variant.id === variantId)
			: undefined;
	const totalSize =
		imported?.format === "mlx"
			? imported.totalSize
			: (selectedVariant?.totalSize ?? 0) +
				(kind === "vlm"
					? (imported?.projectors.find(
							(projector) => projector.path === projectorPath,
						)?.size ?? 0)
					: 0);
	const runtimeFileCount =
		imported?.format === "mlx"
			? imported.assets.length
			: (selectedVariant?.files.length ?? 0) + (kind === "vlm" ? 1 : 0);

	return (
		<Card className="w-full max-w-screen-lg">
			<CardHeader>
				<CardTitle>Import a Hugging Face model</CardTitle>
				<CardDescription>
					Paste a public MLX or GGUF repository. Flow-Like pins its immutable
					snapshot, selects the runtime files, and fills the Bit form.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				<div className="flex flex-col gap-2 sm:flex-row">
					<Input
						id="hf-mlx-repository"
						value={repository}
						disabled={disabled || inspecting}
						placeholder="https://huggingface.co/owner/model"
						onChange={(event) => {
							setRepository(event.target.value);
							setError(null);
						}}
						onPaste={(event) => {
							const pasted = event.clipboardData.getData("text").trim();
							if (!pasted || disabled || inspecting) return;
							event.preventDefault();
							setRepository(pasted);
							void inspectRepository(pasted);
						}}
						onKeyDown={(event) => {
							if (event.key === "Enter") {
								event.preventDefault();
								void inspectRepository();
							}
						}}
					/>
					<Button
						type="button"
						className="shrink-0 gap-2"
						disabled={disabled || inspecting || !repository.trim()}
						onClick={() => void inspectRepository()}
					>
						{inspecting ? (
							<Loader2 className="h-4 w-4 animate-spin" />
						) : (
							<Search className="h-4 w-4" />
						)}
						Inspect &amp; fill
					</Button>
				</div>

				{error ? <p className="text-sm text-destructive">{error}</p> : null}

				{imported ? (
					<div className="space-y-3 rounded-lg border bg-muted/25 p-3">
						<div className="flex flex-wrap items-center gap-2">
							<CheckCircle2 className="h-4 w-4 text-emerald-500" />
							<span className="font-medium">{imported.repoId}</span>
							<Badge variant="default">{imported.format.toUpperCase()}</Badge>
							<Badge variant="secondary">
								{runtimeFileCount} runtime{" "}
								{runtimeFileCount === 1 ? "file" : "files"}
							</Badge>
							<Badge variant="secondary">{humanFileSize(totalSize)}</Badge>
							<Badge variant="outline">{imported.license}</Badge>
							<Badge variant="outline">{imported.revision.slice(0, 10)}</Badge>
						</div>
						{imported.format === "gguf" ? (
							<div className="grid gap-3 sm:grid-cols-2">
								<div className="space-y-1.5">
									<Label htmlFor="hf-gguf-variant">Quantization</Label>
									<Select
										value={variantId}
										onValueChange={(nextVariantId) => {
											setVariantId(nextVariantId);
											applyGgufSelection(imported, {
												variantId: nextVariantId,
												projectorPath,
												kind,
											});
										}}
									>
										<SelectTrigger id="hf-gguf-variant">
											<SelectValue placeholder="Choose a GGUF file" />
										</SelectTrigger>
										<SelectContent>
											{imported.variants.map((variant) => (
												<SelectItem
													key={variant.id}
													value={variant.id}
													disabled={!variant.complete || variant.split}
												>
													{variant.label} · {humanFileSize(variant.totalSize)}
													{variant.split ? " · split (unsupported)" : ""}
												</SelectItem>
											))}
										</SelectContent>
									</Select>
								</div>
								{kind === "vlm" ? (
									<div className="space-y-1.5">
										<Label htmlFor="hf-gguf-projector">Vision projector</Label>
										<Select
											value={projectorPath}
											onValueChange={(nextProjectorPath) => {
												setProjectorPath(nextProjectorPath);
												applyGgufSelection(imported, {
													variantId,
													projectorPath: nextProjectorPath,
													kind,
												});
											}}
										>
											<SelectTrigger id="hf-gguf-projector">
												<SelectValue placeholder="Choose an mmproj file" />
											</SelectTrigger>
											<SelectContent>
												{imported.projectors.map((projector) => (
													<SelectItem
														key={projector.path}
														value={projector.path}
													>
														{projector.path} · {humanFileSize(projector.size)}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
									</div>
								) : null}
							</div>
						) : null}
						<div className="flex flex-wrap items-center gap-2 text-sm">
							<span className="text-muted-foreground">Use as:</span>
							<Button
								type="button"
								size="sm"
								variant={kind === "llm" ? "default" : "outline"}
								onClick={() => overrideKind("llm")}
							>
								LLM
							</Button>
							<Button
								type="button"
								size="sm"
								variant={kind === "vlm" ? "default" : "outline"}
								disabled={
									imported.format === "mlx"
										? !imported.assets.some((asset) =>
												/^(processor_config|preprocessor_config)\.json$/i.test(
													asset.path,
												),
											)
										: imported.projectors.length === 0
								}
								onClick={() => overrideKind("vlm")}
							>
								VLM
							</Button>
							<span className="text-xs text-muted-foreground">
								{imported.kindEvidence.join(" · ")}
							</span>
						</div>
						<p className="text-xs text-muted-foreground">
							{imported.ignoredPaths.length > 0
								? `${imported.ignoredPaths.length} documentation or incompatible files were ignored. `
								: ""}
							Submitting this form asks the backend to copy the selected, pinned
							files from Hugging Face into the shared CDN Bit store.
						</p>
						{imported.warnings.map((warning) => (
							<p key={warning} className="text-xs text-amber-600">
								{warning}
							</p>
						))}
					</div>
				) : (
					<p className="text-xs text-muted-foreground">
						Private and gated repositories are intentionally not accepted
						because this admin flow republishes model bytes to the CDN.
					</p>
				)}
			</CardContent>
		</Card>
	);
}

export function MlxAssetsConfiguration({
	assets,
	setAssets,
	createAsset,
}: Readonly<{
	assets: IBit[];
	setAssets: Dispatch<SetStateAction<IBit[]>>;
	createAsset: (fileName?: string) => IBit;
}>) {
	const updateAsset = (index: number, update: Partial<IBit>) => {
		setAssets((current) =>
			current.map((asset, assetIndex) =>
				assetIndex === index ? { ...asset, ...update } : asset,
			),
		);
	};

	const resolveSize = async (index: number, downloadLink: string) => {
		if (!downloadLink) return;
		try {
			const size = await getModelSize(downloadLink);
			updateAsset(index, { size });
		} catch (error) {
			console.warn("Failed to resolve MLX asset size:", error);
		}
	};

	return (
		<Card className="w-full max-w-screen-lg">
			<CardHeader>
				<div className="flex items-start justify-between gap-4">
					<div className="space-y-1.5">
						<CardTitle>MLX Model Bundle</CardTitle>
						<CardDescription>
							The LLM/VLM entry is a virtual root. Add every Hugging Face
							repository file as a dependency and preserve its model-relative
							path.
						</CardDescription>
					</div>
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="shrink-0 gap-1.5"
						onClick={() => setAssets((current) => [...current, createAsset()])}
					>
						<Plus className="h-3.5 w-3.5" />
						Add file
					</Button>
				</div>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="rounded-md border border-dashed bg-muted/35 px-3 py-2 text-xs text-muted-foreground">
					Required at the root: <code>config.json</code>,{" "}
					<code>tokenizer.json</code>, and <code>tokenizer_config.json</code>,
					plus one or more <code>.safetensors</code> files and a processor
					config for VLMs. Other paths may contain directories, for example{" "}
					<code>weights/model-00001-of-00002.safetensors</code>.
				</div>

				{assets.map((asset, index) => {
					const fileName = asset.file_name ?? "";
					const pathError = fileName
						? mlxAssetPathError(fileName.trim())
						: undefined;
					const inferredType = inferMlxAssetBitType(fileName);
					return (
						<div
							key={asset.id}
							className="space-y-3 rounded-lg border bg-card p-4"
						>
							<div className="flex items-center justify-between gap-2">
								<div className="flex items-center gap-2">
									<span className="text-sm font-medium">File {index + 1}</span>
									<Badge variant="secondary">{inferredType}</Badge>
									{typeof asset.size === "number" && asset.size > 0 ? (
										<span className="text-xs text-muted-foreground">
											{humanFileSize(asset.size)}
										</span>
									) : null}
								</div>
								<Button
									type="button"
									variant="ghost"
									size="icon"
									aria-label={`Remove MLX file ${index + 1}`}
									onClick={() =>
										setAssets((current) =>
											current.filter((_, assetIndex) => assetIndex !== index),
										)
									}
								>
									<Trash2 className="h-4 w-4" />
								</Button>
							</div>
							<div className="grid gap-3 md:grid-cols-2">
								<div className="space-y-2">
									<Label htmlFor={`mlx-path-${asset.id}`}>Stored path *</Label>
									<Input
										id={`mlx-path-${asset.id}`}
										value={fileName}
										placeholder="config.json or weights/model.safetensors"
										onChange={(event) => {
											const nextFileName = event.target.value;
											updateAsset(index, {
												file_name: nextFileName,
												type: inferMlxAssetBitType(nextFileName),
											});
										}}
										onBlur={(event) =>
											updateAsset(index, {
												file_name: event.target.value.trim(),
											})
										}
										required
									/>
									{pathError ? (
										<p className="text-xs text-destructive">{pathError}</p>
									) : (
										<p className="text-xs text-muted-foreground">
											Relative to the materialized MLX model directory.
										</p>
									)}
								</div>
								<div className="space-y-2">
									<Label htmlFor={`mlx-url-${asset.id}`}>Download URL *</Label>
									<Input
										id={`mlx-url-${asset.id}`}
										value={asset.download_link ?? ""}
										placeholder="https://huggingface.co/…/resolve/main/config.json"
										onChange={(event) => {
											const downloadLink = event.target.value.trim();
											const urlFileName =
												downloadLink.split("/").pop()?.split("?")[0] ?? "";
											updateAsset(index, {
												download_link: downloadLink,
												...(fileName ? {} : { file_name: urlFileName }),
											});
										}}
										onBlur={(event) =>
											void resolveSize(index, event.target.value.trim())
										}
										required
									/>
								</div>
							</div>
						</div>
					);
				})}

				{assets.length === 0 ? (
					<div className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
						Add the files that make up this MLX model bundle.
					</div>
				) : null}
			</CardContent>
		</Card>
	);
}
