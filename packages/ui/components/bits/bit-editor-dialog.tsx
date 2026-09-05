"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
	Check,
	FileText,
	FolderOpen,
	ImageIcon,
	Info,
	Loader2,
	LockKeyhole,
	Save,
	Settings2,
	SlidersHorizontal,
	Trash2,
	X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke } from "../../hooks/use-invoke";
import { type IBit, IBitTypes, type IMetadata } from "../../lib/schema/bit/bit";
import { useBackend } from "../../state/backend-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import {
	BitImage,
	EditorField,
	EditorSection,
	ImageField,
	StringList,
} from "./bit-editor-fields";
import {
	SECRET_KEYS,
	clone,
	coreChanged,
	emptyMetadata,
	record,
	saveAdminBit,
	splitBitSecrets,
	validateBitDraft,
} from "./bit-editor-model";
import { BitParametersEditor } from "./bit-parameters-editor";

const sections = [
	{
		id: "details",
		label: "Details",
		hint: "Name, description & tags",
		icon: FileText,
	},
	{
		id: "images",
		label: "Images",
		hint: "Icon, cover & previews",
		icon: ImageIcon,
	},
	{
		id: "parameters",
		label: "Parameters",
		hint: "Connection & behavior",
		icon: SlidersHorizontal,
	},
	{
		id: "files",
		label: "Files & sources",
		hint: "Downloads & dependencies",
		icon: FolderOpen,
	},
	{
		id: "advanced",
		label: "Advanced",
		hint: "Identity & maintenance",
		icon: Settings2,
	},
] as const;
type Section = (typeof sections)[number]["id"];

export function BitEditorDialog({
	bit,
	open,
	onOpenChange,
	scope = "custom",
	onSaved,
	onDeleted,
}: {
	bit: IBit;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	scope?: "custom" | "admin";
	onSaved?: (bit: IBit) => void;
	onDeleted?: () => void;
}) {
	return open ? (
		<BitEditorSession
			key={bit.id}
			bit={bit}
			onOpenChange={onOpenChange}
			scope={scope}
			onSaved={onSaved}
			onDeleted={onDeleted}
		/>
	) : null;
}
function BitEditorSession({
	bit,
	onOpenChange,
	scope,
	onSaved,
	onDeleted,
}: Omit<Parameters<typeof BitEditorDialog>[0], "open"> & {
	scope: "custom" | "admin";
}) {
	const backend = useBackend();
	const queries = useQueryClient();
	const invalidate = useInvalidateInvoke();
	const [initial] = useState(() =>
		scope === "custom"
			? splitBitSecrets(bit)
			: { bit: clone(bit), secrets: {} as Record<string, unknown> },
	);
	const [draft, setDraft] = useState(initial.bit);
	const [baseline, setBaseline] = useState(initial.bit);
	const [secrets, setSecrets] = useState<Record<string, unknown>>(
		initial.secrets,
	);
	const [savedSecrets, setSavedSecrets] = useState(initial.secrets);
	const [section, setSection] = useState<Section>("details");
	const [language, setLanguage] = useState(
		bit.meta?.en ? "en" : (Object.keys(bit.meta ?? {})[0] ?? "en"),
	);
	const [newLanguage, setNewLanguage] = useState("");
	const [showLanguageInput, setShowLanguageInput] = useState(false);
	const [jsonText, setJsonText] = useState<string | null>(null);
	const [jsonError, setJsonError] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const [imagesBusy, setImagesBusy] = useState(0);
	const [error, setError] = useState<string | null>(null);
	const [confirm, setConfirm] = useState<"close" | "reset" | "delete" | null>(
		null,
	);
	const [resetKey, setResetKey] = useState(0);
	const lock = useRef(false);
	const meta = draft.meta?.[language] ?? emptyMetadata();
	const providerName = String(
		record(record(draft.parameters).provider).provider_name ?? "",
	);
	const mlx =
		providerName.toLowerCase() === "mlx" &&
		[IBitTypes.Llm, IBitTypes.Vlm].includes(draft.type);
	const hosted = /^(hosted(?::|$)|premium$|internal$)/i.test(providerName);
	const dirty =
		JSON.stringify(draft) !== JSON.stringify(baseline) ||
		JSON.stringify(secrets) !== JSON.stringify(savedSecrets) ||
		jsonText !== null;
	const blocked = busy || imagesBusy > 0;
	const update = <K extends keyof IBit>(key: K, value: IBit[K]) =>
		setDraft((current) => ({ ...current, [key]: value }));
	const updateMeta = <K extends keyof IMetadata>(key: K, value: IMetadata[K]) =>
		setDraft((current) => ({
			...current,
			meta: {
				...current.meta,
				[language]: {
					...(current.meta?.[language] ?? emptyMetadata()),
					[key]: value,
				},
			},
		}));
	const imageBusy = useCallback(
		(value: boolean) =>
			setImagesBusy((count) => Math.max(0, count + (value ? 1 : -1))),
		[],
	);

	useEffect(() => {
		if (!dirty && !blocked) return;
		const guard = (e: BeforeUnloadEvent) => {
			e.preventDefault();
			e.returnValue = "";
		};
		window.addEventListener("beforeunload", guard);
		return () => window.removeEventListener("beforeunload", guard);
	}, [dirty, blocked]);

	function close() {
		if (blocked) return;
		if (dirty) setConfirm("close");
		else onOpenChange(false);
	}
	function reset() {
		setDraft(clone(baseline));
		if (!baseline.meta[language])
			setLanguage(
				baseline.meta.en ? "en" : (Object.keys(baseline.meta)[0] ?? "en"),
			);
		setSecrets(clone(savedSecrets));
		setJsonText(null);
		setJsonError(null);
		setError(null);
		setResetKey((key) => key + 1);
	}
	async function refreshConsumers() {
		await Promise.allSettled([
			queries.invalidateQueries({ queryKey: ["bit-search"] }),
			queries.invalidateQueries({ queryKey: ["bit"] }),
			invalidate(backend.bitState.listCustomBits, []),
			invalidate(backend.bitState.getProfileBits, []),
			invalidate(backend.userState.getSettingsProfile, []),
			invalidate(backend.userState.getProfile, []),
		]);
	}
	async function save() {
		if (lock.current || blocked || !dirty || jsonText !== null) return;
		const validation = validateBitDraft(draft, scope, baseline);
		if (validation) {
			setError(validation);
			return;
		}
		lock.current = true;
		setBusy(true);
		setError(null);
		try {
			let candidate = clone(draft);
			if (mlx && coreChanged(baseline, candidate))
				candidate = {
					...candidate,
					download_link: null,
					file_name: null,
					size: 0,
				};
			if (scope === "admin" && hosted && coreChanged(baseline, candidate))
				candidate = {
					...candidate,
					download_link: null,
					file_name: null,
					size: 0,
				};
			let saved: IBit;
			if (scope === "custom") {
				const split = splitBitSecrets(candidate);
				const credentials = { ...secrets, ...split.secrets };
				if (typeof credentials.headers === "string") {
					try {
						const headers: unknown = credentials.headers.trim()
							? JSON.parse(credentials.headers)
							: {};
						if (
							!headers ||
							typeof headers !== "object" ||
							Array.isArray(headers) ||
							Object.values(headers).some((value) => typeof value !== "string")
						)
							throw new Error();
						credentials.headers = headers;
					} catch {
						throw new Error(
							"Enter headers as a JSON object with text values in Credentials.",
						);
					}
				}
				saved = await backend.bitState.upsertCustomBit(split.bit, credentials);
				const sanitized = splitBitSecrets(saved);
				saved = sanitized.bit;
				const nextSecrets = {
					...credentials,
					...sanitized.secrets,
				};
				setSecrets(nextSecrets);
				setSavedSecrets(clone(nextSecrets));
			} else {
				const profile = await backend.userState.getProfile();
				saved = await saveAdminBit(
					backend.apiState,
					profile,
					baseline,
					candidate,
					(checkpoint) => {
						setBaseline(checkpoint);
						setDraft((current) => ({ ...checkpoint, meta: current.meta }));
					},
				);
			}
			setBaseline(clone(saved));
			setDraft(clone(saved));
			toast.success("Changes saved");
			onSaved?.(saved);
			await refreshConsumers();
		} catch (e) {
			setError(
				e instanceof Error
					? e.message
					: "Could not save your changes. Please try again.",
			);
		} finally {
			lock.current = false;
			setBusy(false);
		}
	}
	async function remove() {
		if (lock.current || blocked) return;
		lock.current = true;
		setBusy(true);
		setError(null);
		setConfirm(null);
		try {
			if (scope === "custom") await backend.bitState.deleteCustomBit(draft.id);
			else
				await backend.apiState.del(
					await backend.userState.getProfile(),
					`admin/bit/${encodeURIComponent(draft.id)}`,
				);
			await refreshConsumers();
			onDeleted?.();
			onOpenChange(false);
			toast.success("Bit deleted");
		} catch (e) {
			setError(e instanceof Error ? e.message : "Could not delete this bit.");
		} finally {
			lock.current = false;
			setBusy(false);
		}
	}
	async function repair() {
		if (lock.current || dirty || blocked) return;
		lock.current = true;
		setBusy(true);
		setError(null);
		try {
			const pack = await backend.bitState.repairTtsBitAssets(draft);
			const replacement = pack.bits[0];
			if (replacement) {
				setDraft(clone(replacement));
				setBaseline(clone(replacement));
				onSaved?.(replacement);
			}
			await refreshConsumers();
			toast.success("TTS assets repaired");
		} catch (e) {
			setError(e instanceof Error ? e.message : "Could not repair TTS assets.");
		} finally {
			lock.current = false;
			setBusy(false);
		}
	}
	function applyJson() {
		try {
			const value = JSON.parse(jsonText ?? "null");
			if (scope === "custom") {
				const split = splitBitSecrets({ ...draft, parameters: value });
				update("parameters", split.bit.parameters);
				setSecrets((current) => ({ ...current, ...split.secrets }));
			} else update("parameters", value);
			setJsonText(null);
			setJsonError(null);
		} catch (e) {
			setJsonError(e instanceof Error ? e.message : "Enter valid JSON.");
		}
	}
	function addLanguage() {
		const code = newLanguage.trim().toLowerCase();
		if (!/^[a-z]{2,3}(-[a-z0-9]{2,8})*$/.test(code)) {
			setError("Use a language code such as en, de, or pt-br.");
			return;
		}
		if (!draft.meta[code])
			update("meta", { ...draft.meta, [code]: emptyMetadata() });
		setLanguage(code);
		setNewLanguage("");
		setShowLanguageInput(false);
		setError(null);
	}
	return (
		<>
			<Dialog
				open
				onOpenChange={(open) => {
					if (!open) close();
				}}
			>
				<DialogContent
					showCloseButton={false}
					className="h-[min(860px,calc(100dvh-2rem))] gap-0 overflow-hidden p-0 sm:max-w-6xl"
					onKeyDown={(e) => {
						if ((e.metaKey || e.ctrlKey) && e.key === "s") {
							e.preventDefault();
							void save();
						}
					}}
				>
					<header className="flex shrink-0 items-center gap-3 border-b px-5 py-4 sm:px-7">
						<BitImage src={meta.icon} name="Bit icon" className="size-11" />
						<div className="min-w-0 flex-1">
							<div className="flex items-center gap-2">
								<DialogTitle className="truncate text-lg">
									{meta.name || "Untitled bit"}
								</DialogTitle>
								<Badge variant="outline" className="hidden sm:inline-flex">
									{scope === "custom" ? "Your custom bit" : "Registry bit"}
								</Badge>
							</div>
							<DialogDescription className="mt-1 text-xs">
								{scope === "custom"
									? "Edit the model in your personal library."
									: "Edit the published bit in your registry."}
							</DialogDescription>
						</div>
						<Button
							variant="ghost"
							size="icon"
							disabled={blocked}
							aria-label="Close bit editor"
							onClick={close}
						>
							<X className="size-4" />
						</Button>
					</header>
					<div className="flex min-h-0 flex-1 flex-col md:flex-row">
						<nav
							aria-label="Bit editor sections"
							className="flex shrink-0 gap-1 overflow-x-auto border-b bg-muted/20 p-2 md:w-48 md:flex-col md:border-r md:border-b-0 md:p-3 lg:w-52"
						>
							{sections.map((item) => (
								<button
									type="button"
									key={item.id}
									aria-current={section === item.id ? "page" : undefined}
									onClick={() => setSection(item.id)}
									className={`flex shrink-0 items-center gap-3 rounded-lg px-3 py-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${section === item.id ? "bg-primary/10 text-primary" : "text-muted-foreground hover:bg-muted hover:text-foreground"}`}
								>
									<item.icon className="size-4 shrink-0" />
									<span>
										<span className="block whitespace-nowrap text-sm font-medium">
											{item.label}
										</span>
										<span className="mt-0.5 hidden text-[11px] text-muted-foreground md:block">
											{item.hint}
										</span>
									</span>
								</button>
							))}
							<div className="mt-auto hidden px-3 pb-2 pt-8 text-xs leading-relaxed text-muted-foreground md:block">
								<LockKeyhole className="mb-2 size-4" />
								{scope === "custom"
									? "Your configuration stays in your personal library."
									: "Changes apply to the published registry entry."}
							</div>
						</nav>
						<div className="min-h-0 min-w-0 flex-1 overflow-y-auto overscroll-contain">
							<div className="mx-auto grid max-w-5xl gap-8 p-5 sm:p-7 xl:grid-cols-[minmax(0,1fr)_230px]">
								<fieldset
									disabled={blocked}
									key={resetKey}
									className="min-w-0 space-y-8 disabled:opacity-70"
								>
									{(section === "details" || section === "images") && (
										<div className="flex flex-wrap items-end justify-between gap-3 rounded-lg bg-muted/40 p-3">
											<div className="space-y-1.5">
												<label
													htmlFor="metadata-language"
													className="text-xs font-medium text-muted-foreground"
												>
													Metadata language
												</label>
												<select
													id="metadata-language"
													className="block min-w-32 rounded-md border bg-background px-3 py-1.5 text-sm"
													value={language}
													onChange={(e) => setLanguage(e.target.value)}
												>
													{[
														...new Set([
															...Object.keys(draft.meta ?? {}),
															language,
														]),
													].map((code) => (
														<option key={code} value={code}>
															{code === "en"
																? "English (en)"
																: code === "de"
																	? "German (de)"
																	: code}
														</option>
													))}
												</select>
											</div>
											{showLanguageInput ? (
												<div className="flex gap-2">
													<Input
														aria-label="New language code"
														className="w-24"
														placeholder="e.g. fr"
														value={newLanguage}
														onChange={(e) => setNewLanguage(e.target.value)}
													/>
													<Button
														className="bg-foreground text-background hover:bg-foreground/90"
														size="sm"
														onClick={addLanguage}
													>
														Add
													</Button>
												</div>
											) : (
												<Button
													size="sm"
													variant="ghost"
													onClick={() => setShowLanguageInput(true)}
												>
													Add language
												</Button>
											)}
										</div>
									)}
									{section === "details" && (
										<>
											<EditorSection
												title="Make it recognizable"
												description="Help people find this bit and understand when to use it."
											>
												<EditorField
													label="Display name"
													value={meta.name}
													onChange={(value) => updateMeta("name", value)}
													required
													placeholder="Give your bit a clear name"
												/>
												<EditorField
													label="Description"
													multiline
													value={meta.description}
													onChange={(value) => updateMeta("description", value)}
													placeholder="What does this bit do best?"
												/>
												<StringList
													key={`tags-${language}`}
													label="Tags"
													value={meta.tags ?? []}
													onChange={(value) => updateMeta("tags", value)}
													placeholder="Add a tag and press Enter"
												/>
												<EditorField
													label="Use case"
													value={meta.use_case ?? ""}
													onChange={(value) => updateMeta("use_case", value)}
													placeholder="For example, searching technical documentation"
												/>
											</EditorSection>
											<div className="border-t" />
											<EditorSection title="More about this bit">
												<EditorField
													label="Full description"
													multiline
													value={meta.long_description ?? ""}
													onChange={(value) =>
														updateMeta("long_description", value)
													}
													hint="Add setup notes, capabilities, or examples. Markdown is supported in the stored description."
												/>
												<div className="grid gap-4 sm:grid-cols-2">
													<EditorField
														label="Version"
														value={draft.version ?? ""}
														onChange={(value) => update("version", value)}
													/>
													<EditorField
														label="License"
														value={draft.license ?? ""}
														onChange={(value) => update("license", value)}
														placeholder="e.g. Apache-2.0"
													/>
												</div>
												{scope === "admin" && (
													<StringList
														label="Authors"
														value={draft.authors ?? []}
														onChange={(value) => update("authors", value)}
													/>
												)}
												<EditorField
													label="Release notes"
													multiline
													value={meta.release_notes ?? ""}
													onChange={(value) =>
														updateMeta("release_notes", value)
													}
												/>
											</EditorSection>
											<details className="rounded-lg border p-4">
												<summary className="cursor-pointer text-sm font-medium">
													Website & support links
												</summary>
												<div className="mt-5 space-y-4">
													{(
														[
															["website", "Website"],
															["docs_url", "Documentation URL"],
															["support_url", "Support URL"],
														] as const
													).map(([key, label]) => (
														<EditorField
															key={key}
															label={label}
															value={meta[key] ?? ""}
															onChange={(value) => updateMeta(key, value)}
															placeholder="https://"
														/>
													))}
												</div>
											</details>
										</>
									)}
									{section === "images" && (
										<EditorSection
											title="Give it a visual identity"
											description="Choose images or paste image URLs. Your preview updates as you edit."
										>
											<ImageField
												key={`icon-${language}`}
												label="Icon"
												value={meta.icon}
												onChange={(value) => updateMeta("icon", value)}
												onBusyChange={imageBusy}
											/>
											<ImageField
												key={`cover-${language}`}
												label="Cover image"
												wide
												value={meta.thumbnail}
												onChange={(value) => updateMeta("thumbnail", value)}
												onBusyChange={imageBusy}
											/>
											<StringList
												key={`media-${language}`}
												label="Preview media URLs"
												value={meta.preview_media ?? []}
												onChange={(value) => updateMeta("preview_media", value)}
												placeholder="Paste an image or video URL"
											/>
											{!!meta.preview_media?.length && (
												<div className="grid grid-cols-2 gap-3">
													{meta.preview_media.map((url, i) => (
														<BitImage
															key={`${i}:${url}`}
															src={url}
															name={`Media preview ${i + 1}`}
															className="aspect-video w-full"
														/>
													))}
												</div>
											)}
										</EditorSection>
									)}
									<div hidden={section !== "parameters"} className="space-y-8">
										<BitParametersEditor
											scope={scope}
											value={draft.parameters}
											bitType={draft.type}
											onChange={(value) => update("parameters", value)}
											jsonText={jsonText}
											onJsonChange={(text) => {
												setJsonText(text);
												setJsonError(null);
											}}
											jsonError={jsonError}
											onApplyJson={applyJson}
											onResetJson={() => {
												setJsonText(null);
												setJsonError(null);
											}}
										/>
										{scope === "custom" && (
											<details className="rounded-xl border p-4">
												<summary className="cursor-pointer text-sm font-medium">
													Credentials
												</summary>
												<div className="mt-5 space-y-4">
													<p className="text-xs text-muted-foreground">
														Existing credentials are kept when left unchanged.
													</p>
													{SECRET_KEYS.map((key) => (
														<EditorField
															key={key}
															label={
																key === "api_key"
																	? "API key"
																	: key.replaceAll("_", " ")
															}
															type="password"
															value={
																typeof secrets[key] === "string"
																	? (secrets[key] as string)
																	: secrets[key]
																		? JSON.stringify(secrets[key])
																		: ""
															}
															placeholder="Leave unchanged"
															onChange={(value) =>
																setSecrets((current) => ({
																	...current,
																	[key]: value,
																}))
															}
														/>
													))}
												</div>
											</details>
										)}
									</div>
									{section === "files" && (
										<EditorSection
											title="Files & sources"
											description="Manage where this bit comes from and the files it needs."
										>
											<EditorField
												label="Repository URL"
												value={draft.repository ?? ""}
												onChange={(value) => update("repository", value)}
												placeholder="https://huggingface.co/organization/model"
											/>
											{mlx || hosted ? (
												<div className="flex gap-3 rounded-lg border bg-muted/30 p-4 text-sm leading-relaxed text-muted-foreground">
													<Info className="mt-0.5 size-4 shrink-0" />
													<p>
														{mlx
															? scope === "custom"
																? "This MLX model uses the Hugging Face file manifest in Parameters."
																: "This MLX model loads its files through the dependencies below."
															: "This model runs through a hosted provider. Configure its connection in Parameters."}
													</p>
												</div>
											) : (
												<>
													<EditorField
														label="Download URL"
														value={draft.download_link ?? ""}
														onChange={(value) => update("download_link", value)}
														placeholder="https://"
													/>
													<EditorField
														label="File name"
														value={draft.file_name ?? ""}
														onChange={(value) => update("file_name", value)}
													/>
													<EditorField
														label="File size (bytes)"
														type="number"
														value={draft.size ?? 0}
														onChange={(value) => update("size", Number(value))}
														disabled={
															scope === "admin" &&
															draft.download_link === baseline.download_link
														}
														hint={
															scope === "admin"
																? "The registry calculates file size when downloading an artifact."
																: "The size of the downloadable model file."
														}
													/>
												</>
											)}
											{scope === "admin" && (
												<StringList
													label="Dependencies"
													value={draft.dependencies ?? []}
													onChange={(value) => update("dependencies", value)}
													placeholder="Add a bit reference and press Enter"
												/>
											)}
										</EditorSection>
									)}
									{section === "advanced" && (
										<>
											<EditorSection
												title="Identity"
												description="Reference details and settings for this bit."
											>
												<EditorField
													label="Bit ID"
													value={draft.id}
													onChange={() => {}}
													disabled
												/>
												<div className="space-y-2">
													<label
														htmlFor="edit-bit-type"
														className="text-sm font-medium"
													>
														Bit type
													</label>
													<select
														id="edit-bit-type"
														className="h-9 w-full rounded-md border bg-background px-3 text-sm"
														value={draft.type}
														onChange={(e) =>
															update("type", e.target.value as IBitTypes)
														}
													>
														{(scope === "custom"
															? [IBitTypes.Llm, IBitTypes.Vlm]
															: Object.values(IBitTypes)
														).map((type) => (
															<option key={type} value={type}>
																{type}
															</option>
														))}
													</select>
												</div>
												{scope === "admin" && (
													<EditorField
														label="Model slug"
														value={draft.model_slug ?? ""}
														onChange={(value) => update("model_slug", value)}
														hint="Identifier used to match model evaluations."
													/>
												)}
												<EditorField
													label="Hub"
													value={draft.hub}
													onChange={() => {}}
													disabled
												/>
											</EditorSection>
											{scope === "admin" && draft.type === IBitTypes.Tts && (
												<EditorSection title="Maintenance">
													<p className="text-sm text-muted-foreground">
														Repair missing TTS assets after saving your changes.
													</p>
													<Button
														variant="outline"
														disabled={dirty}
														onClick={() => void repair()}
													>
														Repair TTS assets
													</Button>
												</EditorSection>
											)}
											<div className="space-y-3 rounded-xl border border-destructive/25 p-5">
												<h3 className="text-sm font-semibold">
													Delete this bit
												</h3>
												<p className="text-sm leading-relaxed text-muted-foreground">
													{scope === "custom"
														? "Remove this bit and its credentials from your library. Flows using it will need another model."
														: "Remove this registry entry and its stored artifact. This cannot be undone."}
												</p>
												<Button
													variant="outline"
													className="border-destructive/30 text-destructive hover:bg-destructive/10"
													onClick={() => setConfirm("delete")}
												>
													<Trash2 className="size-4" />
													Delete bit
												</Button>
											</div>
										</>
									)}
								</fieldset>
								<aside className="hidden xl:block">
									<div className="sticky top-0 space-y-4">
										<p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
											Live preview
										</p>
										<div className="overflow-hidden rounded-2xl border bg-card shadow-sm">
											{meta.thumbnail ? (
												<BitImage
													src={meta.thumbnail}
													name="Cover preview"
													className="aspect-[16/9] w-full rounded-none"
												/>
											) : (
												<div className="h-20 bg-gradient-to-br from-primary/15 via-primary/5 to-muted" />
											)}
											<div className="space-y-3 p-4">
												<BitImage
													src={meta.icon}
													name="Icon preview"
													className="-mt-9 size-14 border-4 border-card bg-card"
												/>
												<Badge variant="secondary" className="text-[10px]">
													{draft.type}
												</Badge>
												<h3 className="break-words font-semibold">
													{meta.name || "Your bit name"}
												</h3>
												<p className="line-clamp-5 break-words text-xs leading-relaxed text-muted-foreground">
													{meta.description ||
														"A short description helps people choose the right bit."}
												</p>
												<div className="flex flex-wrap gap-1">
													{(meta.tags ?? []).slice(0, 4).map((tag) => (
														<Badge
															key={tag}
															variant="outline"
															className="max-w-full break-all text-[10px]"
														>
															{tag}
														</Badge>
													))}
												</div>
											</div>
										</div>
										<p className="text-xs leading-relaxed text-muted-foreground">
											{language.toUpperCase()} metadata preview. Save to apply
											your changes.
										</p>
									</div>
								</aside>
							</div>
						</div>
					</div>
					<footer className="shrink-0 border-t bg-background px-5 py-4 sm:px-7">
						{error && (
							<p role="alert" className="mb-3 text-sm text-destructive">
								{error}
							</p>
						)}
						{jsonText !== null && (
							<p className="mb-3 text-sm text-amber-600 dark:text-amber-400">
								Apply or discard your JSON edits in{" "}
								<button
									type="button"
									className="underline"
									onClick={() => setSection("parameters")}
								>
									Parameters
								</button>{" "}
								before saving.
							</p>
						)}
						<div className="flex items-center justify-between gap-3">
							<output
								aria-live="polite"
								className="flex items-center gap-2 text-xs text-muted-foreground"
							>
								{blocked ? (
									<Loader2 className="size-3.5 animate-spin" />
								) : dirty ? (
									<span className="size-1.5 rounded-full bg-amber-500" />
								) : (
									<Check className="size-3.5 text-emerald-600" />
								)}
								<span>
									{busy
										? "Saving changes…"
										: imagesBusy
											? "Preparing image…"
											: dirty
												? "Unsaved changes"
												: "All changes saved"}
								</span>
							</output>
							<div className="flex gap-2">
								<Button
									variant="ghost"
									size="sm"
									disabled={!dirty || blocked}
									onClick={() => setConfirm("reset")}
								>
									Discard
								</Button>
								<Button
									className="bg-foreground text-background hover:bg-foreground/90"
									size="sm"
									disabled={!dirty || blocked || jsonText !== null}
									onClick={() => void save()}
								>
									{busy ? (
										<Loader2 className="size-4 animate-spin" />
									) : (
										<Save className="size-4" />
									)}
									Save changes
								</Button>
							</div>
						</div>
					</footer>
				</DialogContent>
			</Dialog>
			<AlertDialog
				open={confirm !== null}
				onOpenChange={(open) => {
					if (!open) setConfirm(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{confirm === "delete"
								? `Delete ${meta.name || "this bit"}?`
								: "Discard unsaved changes?"}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{confirm === "delete"
								? "This removes the bit and cannot be undone."
								: "Your edits have not been saved. Keep editing to continue working on them."}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{confirm === "delete" ? "Cancel" : "Keep editing"}
						</AlertDialogCancel>
						<AlertDialogAction
							className="bg-foreground text-background hover:bg-foreground/90"
							onClick={() => {
								if (confirm === "delete") void remove();
								else if (confirm === "close") onOpenChange(false);
								else reset();
								setConfirm(null);
							}}
						>
							{confirm === "delete" ? "Delete bit" : "Discard changes"}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}
