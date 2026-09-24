"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	CardHeader,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Skeleton,
	Textarea,
} from "@flow-like/flow-like-ui";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import {
	AlertCircle,
	AlertTriangle,
	FileText,
	Globe,
	HardDrive,
	Loader2,
	Lock,
	Package,
	Plus,
	RefreshCw,
	Save,
	Trash2,
	Zap,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { isPlaceholderId } from "../mine-model";
import { mineQueryKeys } from "../use-mine-packages";
import { nodeInspectionKey } from "./node-debugger";

const MEMORY_TIERS = [
	{ value: "minimal", label: "Minimal (16 MB)" },
	{ value: "light", label: "Light (32 MB)" },
	{ value: "standard", label: "Standard (64 MB)" },
	{ value: "heavy", label: "Heavy (128 MB)" },
	{ value: "intensive", label: "Intensive (256 MB)" },
	{ value: "large", label: "Large (512 MB)" },
	{ value: "huge", label: "Huge (1 GB)" },
	{ value: "extreme", label: "Extreme (2 GB)" },
	{ value: "maximum", label: "Maximum (4 GB)" },
];

const TIMEOUT_TIERS = [
	{ value: "quick", label: "Quick (5s)" },
	{ value: "standard", label: "Standard (30s)" },
	{ value: "extended", label: "Extended (60s)" },
	{ value: "long_running", label: "Long Running (5min)" },
	{ value: "very_long", label: "Very Long (10min)" },
	{ value: "maximum", label: "Maximum (30min)" },
];

interface ManifestData {
	manifest_version: number;
	id: string;
	name: string;
	version: string;
	description: string;
	authors: { name: string; email?: string; url?: string }[];
	license?: string;
	repository?: string;
	homepage?: string;
	keywords: string[];
	// Capability flags are not authored: nodes declare them in code and the
	// registry derives the store listing from the compiled nodes.
	permissions: {
		memory: string;
		timeout: string;
		network: {
			allowed_hosts: string[];
		};
		oauth_scopes: {
			provider: string;
			scopes: string[];
			reason: string;
			required: boolean;
		}[];
	};
}

function createDefaultManifest(): ManifestData {
	return {
		manifest_version: 1,
		id: "com.example.my-package",
		name: i18next.t("myPackage", "My Package"),
		version: "0.1.0",
		description: "",
		authors: [{ name: "" }],
		license: "MIT",
		repository: "",
		homepage: "",
		keywords: [],
		permissions: {
			memory: "standard",
			timeout: "standard",
			network: {
				allowed_hosts: [],
			},
			oauth_scopes: [],
		},
	};
}

const CAPABILITY_KEYS = [
	"filesystem",
	"database",
	"variables",
	"cache",
	"streaming",
	"a2ui",
	"models",
];

const NETWORK_CAPABILITY_KEYS = [
	"http_enabled",
	"websocket_enabled",
	"tcp_enabled",
	"udp_enabled",
	"dns_enabled",
];

function omitKeys<T extends object>(value: T, keys: string[]): T {
	return Object.fromEntries(
		Object.entries(value).filter(([key]) => !keys.includes(key)),
	) as T;
}

/** Drops the capability flags an older manifest still authors. */
function stripCapabilityFlags(manifest: ManifestData): ManifestData {
	const permissions = manifest.permissions;
	if (!permissions) return manifest;
	return {
		...manifest,
		permissions: {
			...omitKeys(permissions, CAPABILITY_KEYS),
			network: {
				...omitKeys(permissions.network ?? {}, NETWORK_CAPABILITY_KEYS),
				allowed_hosts: permissions.network?.allowed_hosts ?? [],
			},
		},
	};
}

function SectionHeader({
	icon: Icon,
	title,
	description,
}: {
	icon: React.ComponentType<{ className?: string }>;
	title: string;
	description?: string;
}) {
	return (
		<div className="flex items-center gap-2 pb-2">
			<Icon className="h-4 w-4 text-primary" />
			<div>
				<h3 className="text-sm font-semibold">{title}</h3>
				{description && (
					<p className="text-xs text-muted-foreground">{description}</p>
				)}
			</div>
		</div>
	);
}

function IdentitySection({
	data,
	onChange,
	focusId,
	flagPlaceholderId,
}: {
	data: ManifestData;
	onChange: (d: ManifestData) => void;
	focusId: boolean;
	flagPlaceholderId: boolean;
}) {
	const { t } = useTranslation("common");
	const idRef = useRef<HTMLInputElement>(null);
	const placeholder = flagPlaceholderId && isPlaceholderId(data.id);

	useEffect(() => {
		if (focusId) idRef.current?.focus();
	}, [focusId]);

	return (
		<Card>
			<CardHeader className="pb-3">
				<SectionHeader
					icon={Package}
					title={t("packageIdentity", "Package Identity")}
					description={t(
						"coreMetadataForYourPackage",
						"Core metadata for your package",
					)}
				/>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
					<div className="space-y-1.5">
						<Label htmlFor="manifest-id" className="text-xs">
							{t("packageId2", "Package ID")}
						</Label>
						<Input
							id="manifest-id"
							ref={idRef}
							value={data.id}
							onChange={(e) => onChange({ ...data, id: e.target.value })}
							placeholder="com.example.my-package"
							aria-describedby={
								placeholder ? "manifest-id-placeholder" : undefined
							}
							className="h-9 font-mono text-xs"
						/>
						{placeholder && (
							<p
								id="manifest-id-placeholder"
								className="flex items-start gap-1.5 text-xs text-tertiary"
							>
								<AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
								{t(
									"placeholderIdHint",
									"com.example.* is the template's placeholder. Use a reverse domain you control, e.g. com.yourname.package.",
								)}
							</p>
						)}
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs">{t("version", "Version")}</Label>
						<Input
							value={data.version}
							onChange={(e) => onChange({ ...data, version: e.target.value })}
							placeholder="0.1.0"
							className="h-9 font-mono text-xs"
						/>
					</div>
				</div>
				<div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
					<div className="space-y-1.5">
						<Label className="text-xs">Name</Label>
						<Input
							value={data.name}
							onChange={(e) => onChange({ ...data, name: e.target.value })}
							className="h-9"
						/>
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs">{t("license", "License")}</Label>
						<Input
							value={data.license ?? ""}
							onChange={(e) =>
								onChange({
									...data,
									license: e.target.value || undefined,
								})
							}
							placeholder="MIT"
							className="h-9"
						/>
					</div>
				</div>
				<div className="space-y-1.5">
					<Label className="text-xs">{t("description", "Description")}</Label>
					<Textarea
						value={data.description}
						onChange={(e) => onChange({ ...data, description: e.target.value })}
						rows={2}
						className="text-sm"
						placeholder={t(
							"whatDoesThisPackageDo",
							"What does this package do?",
						)}
					/>
				</div>
				<div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
					<div className="space-y-1.5">
						<Label className="text-xs">{t("repository", "Repository")}</Label>
						<Input
							value={data.repository ?? ""}
							onChange={(e) =>
								onChange({
									...data,
									repository: e.target.value || undefined,
								})
							}
							placeholder="https://github.com/..."
							className="h-9"
						/>
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs">{t("homepage", "Homepage")}</Label>
						<Input
							value={data.homepage ?? ""}
							onChange={(e) =>
								onChange({
									...data,
									homepage: e.target.value || undefined,
								})
							}
							placeholder="https://..."
							className="h-9"
						/>
					</div>
				</div>
				<div className="space-y-1.5">
					<Label className="text-xs">
						{t("keywordsCommaseparated", "Keywords (comma-separated)")}
					</Label>
					<Input
						value={data.keywords.join(", ")}
						onChange={(e) =>
							onChange({
								...data,
								keywords: e.target.value
									.split(",")
									.map((k) => k.trim())
									.filter(Boolean),
							})
						}
						placeholder={t("aiTransformData", "ai, transform, data")}
						className="h-9"
					/>
				</div>
				<AuthorsEditor
					authors={data.authors}
					onChange={(authors) => onChange({ ...data, authors })}
				/>
			</CardContent>
		</Card>
	);
}

function AuthorsEditor({
	authors,
	onChange,
}: {
	authors: ManifestData["authors"];
	onChange: (a: ManifestData["authors"]) => void;
}) {
	const { t } = useTranslation("common");
	const addAuthor = () => onChange([...authors, { name: "" }]);
	const removeAuthor = (i: number) =>
		onChange(authors.filter((_, idx) => idx !== i));
	const updateAuthor = (i: number, field: string, value: string) => {
		const updated = [...authors];
		updated[i] = { ...updated[i], [field]: value || undefined };
		onChange(updated);
	};

	return (
		<div className="space-y-2">
			<div className="flex items-center justify-between">
				<Label className="text-xs">{t("authors", "Authors")}</Label>
				<Button
					variant="ghost"
					size="sm"
					onClick={addAuthor}
					className="h-7 text-xs"
				>
					<Plus className="h-3 w-3 mr-1" />
					{t("add", "Add")}
				</Button>
			</div>
			{authors.map((author, i) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: authors are positional manifest rows without an id
				<div key={`author-${i}`} className="flex items-center gap-2">
					<Input
						value={author.name}
						onChange={(e) => updateAuthor(i, "name", e.target.value)}
						placeholder="Name"
						className="h-8 text-xs flex-1"
					/>
					<Input
						value={author.email ?? ""}
						onChange={(e) => updateAuthor(i, "email", e.target.value)}
						placeholder="Email"
						className="h-8 text-xs flex-1"
					/>
					{authors.length > 1 && (
						<Button
							variant="ghost"
							size="icon"
							className="h-8 w-8 shrink-0 text-destructive"
							aria-label={t("remove", "Remove")}
							onClick={() => removeAuthor(i)}
						>
							<Trash2 className="h-3 w-3" />
						</Button>
					)}
				</div>
			))}
		</div>
	);
}

function TierSelect({
	label,
	value,
	tiers,
	onChange,
}: {
	label: string;
	value: string;
	tiers: readonly { value: string; label: string }[];
	onChange: (value: string) => void;
}) {
	return (
		<div className="space-y-1.5">
			<Label className="text-xs">{label}</Label>
			<Select value={value} onValueChange={onChange}>
				<SelectTrigger className="h-9">
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					{tiers.map((tier) => (
						<SelectItem key={tier.value} value={tier.value}>
							{tier.label}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);
}

function PermissionsSection({
	data,
	onChange,
}: {
	data: ManifestData;
	onChange: (d: ManifestData) => void;
}) {
	const { t } = useTranslation("common");
	const p = data.permissions ?? {};
	const net = p.network ?? { allowed_hosts: [] };
	const updatePerm = (patch: Partial<ManifestData["permissions"]>) =>
		onChange({
			...data,
			permissions: { ...p, network: net, ...patch },
		});

	return (
		<Card>
			<CardHeader className="pb-3">
				<SectionHeader
					icon={Lock}
					title="Permissions"
					description={t(
						"setTheResourceLimitsAndOutboundHostsForYourPackage",
						"Set the resource limits and outbound hosts for your package",
					)}
				/>
			</CardHeader>
			<CardContent className="space-y-5">
				<div className="space-y-3">
					<div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
						<HardDrive className="h-3 w-3" /> {t("resources", "Resources")}
					</div>
					<div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
						<TierSelect
							label={t("memory", "Memory")}
							value={p.memory}
							tiers={MEMORY_TIERS}
							onChange={(memory) => updatePerm({ memory })}
						/>
						<TierSelect
							label={t("timeout", "Timeout")}
							value={p.timeout}
							tiers={TIMEOUT_TIERS}
							onChange={(timeout) => updatePerm({ timeout })}
						/>
					</div>
				</div>

				<Separator />

				<div className="space-y-3">
					<div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
						<Globe className="h-3 w-3" /> {t("network", "Network")}
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs">
							{t(
								"allowedHostsCommaseparatedEmptyAll",
								"Allowed Hosts (comma-separated, empty = all)",
							)}
						</Label>
						<Input
							value={(net.allowed_hosts ?? []).join(", ")}
							onChange={(e) =>
								updatePerm({
									network: {
										...net,
										allowed_hosts: e.target.value
											.split(",")
											.map((h) => h.trim())
											.filter(Boolean),
									},
								})
							}
							placeholder={t(
								"apiexamplecomCdnexamplecom",
								"api.example.com, cdn.example.com",
							)}
							className="h-8 text-xs"
						/>
					</div>
				</div>

				<Separator />

				<div className="space-y-3">
					<div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
						<Zap className="h-3 w-3" /> {t("capabilities", "Capabilities")}
					</div>
					<p className="text-xs text-muted-foreground">
						{t(
							"capabilitiesComeFromNodes",
							"Capabilities (network, storage, database, models, ...) are declared by each node in code. The sandbox enforces them and the store lists them, so they are not set in flow-like.toml.",
						)}
					</p>
				</div>
			</CardContent>
		</Card>
	);
}

export interface ManifestEditorProps {
	projectPath: string;
	onSaved?: () => void;
	/** Warn about a `com.example.*` id and focus it; off once the caller maintains that id on the registry. */
	flagPlaceholderId?: boolean;
}

/** Unsaved edits per checkout, so they survive the workspace unmounting an inactive tab. */
const unsavedDrafts = new Map<string, ManifestData>();

/** Edits `flow-like.toml` in `projectPath`; loads on mount and whenever the path changes. */
export function ManifestEditor({
	projectPath,
	onSaved,
	flagPlaceholderId = true,
}: Readonly<ManifestEditorProps>) {
	const { t } = useTranslation("common");
	const queryClient = useQueryClient();
	const [data, setData] = useState<ManifestData | null>(null);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [hasChanges, setHasChanges] = useState(false);
	const [loadedPlaceholder, setLoadedPlaceholder] = useState(false);

	const loadManifest = useCallback(async () => {
		unsavedDrafts.delete(projectPath);
		setLoading(true);
		try {
			const raw = await invoke<Record<string, unknown>>(
				"developer_get_manifest",
				{ projectPath },
			);
			const manifest = raw as unknown as ManifestData;
			setData(manifest);
			setLoadedPlaceholder(isPlaceholderId(manifest.id));
			setHasChanges(false);
		} catch {
			setData(null);
		} finally {
			setLoading(false);
		}
	}, [projectPath]);

	useEffect(() => {
		const draft = unsavedDrafts.get(projectPath);
		if (!draft) {
			void loadManifest();
			return;
		}
		setData(draft);
		setLoadedPlaceholder(isPlaceholderId(draft.id));
		setHasChanges(true);
		setLoading(false);
	}, [projectPath, loadManifest]);

	const handleChange = useCallback(
		(updated: ManifestData) => {
			unsavedDrafts.set(projectPath, updated);
			setData(updated);
			setHasChanges(true);
		},
		[projectPath],
	);

	const createNew = useCallback(() => {
		handleChange(createDefaultManifest());
		setLoadedPlaceholder(true);
	}, [handleChange]);

	const saveManifest = useCallback(async () => {
		if (!data) return;
		setSaving(true);
		try {
			await invoke("developer_save_manifest", {
				projectPath,
				manifest: stripCapabilityFlags(data),
			});
			toast.success(t("manifestSaved", "Manifest saved"));
			unsavedDrafts.delete(projectPath);
			setHasChanges(false);
			void queryClient.invalidateQueries({
				queryKey: mineQueryKeys.manifest(projectPath),
			});
			void queryClient.invalidateQueries({
				queryKey: nodeInspectionKey({ projectPath }),
			});
			onSaved?.();
		} catch (err) {
			toast.error(
				t("failedToSaveManifest", "Failed to save: {{error}}", {
					error: String(err),
				}),
			);
		} finally {
			setSaving(false);
		}
	}, [data, projectPath, queryClient, onSaved, t]);

	return (
		<div className="space-y-4">
			<div className="flex flex-wrap items-center gap-2 rounded-xl border border-border/60 bg-card px-4 py-3">
				<FileText className="size-4 shrink-0 text-muted-foreground" />
				<span
					className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground"
					title={projectPath}
				>
					{`${projectPath.replace(/[\\/]+$/, "")}/flow-like.toml`}
				</span>
				{hasChanges && (
					<Badge
						variant="outline"
						className="border-tertiary/40 text-xs text-tertiary"
					>
						{t("unsavedChanges", "Unsaved changes")}
					</Badge>
				)}
				<Button
					variant="outline"
					size="sm"
					onClick={() => void loadManifest()}
					disabled={loading || saving}
				>
					<RefreshCw className={loading ? "animate-spin" : undefined} />
					{t("reload", "Reload")}
				</Button>
				<Button
					onClick={() => void saveManifest()}
					disabled={saving || !data}
					size="sm"
					className="gap-1.5"
				>
					{saving ? (
						<Loader2 className="h-4 w-4 animate-spin" />
					) : (
						<Save className="h-4 w-4" />
					)}
					{t("save", "Save")}
				</Button>
			</div>

			<AnimatePresence mode="wait">
				{data ? (
					<motion.div
						key="editor"
						initial={{ opacity: 0, y: 10 }}
						animate={{ opacity: 1, y: 0 }}
						exit={{ opacity: 0, y: -10 }}
						className="space-y-4"
					>
						<IdentitySection
							data={data}
							onChange={handleChange}
							focusId={flagPlaceholderId && loadedPlaceholder}
							flagPlaceholderId={flagPlaceholderId}
						/>
						<PermissionsSection data={data} onChange={handleChange} />
					</motion.div>
				) : loading ? (
					<div key="loading" className="space-y-4">
						<Skeleton className="h-80 w-full rounded-xl" />
						<Skeleton className="h-56 w-full rounded-xl" />
					</div>
				) : (
					<motion.div
						key="missing"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
					>
						<Card>
							<CardContent className="py-12 text-center space-y-3">
								<AlertCircle className="h-10 w-10 text-muted-foreground mx-auto" />
								<div>
									<p className="font-medium">
										{t("noFlowliketomlFound", "No flow-like.toml found")}
									</p>
									<p className="text-sm text-muted-foreground">
										{t(
											"createANewManifestForThisProject",
											"Create a new manifest for this project",
										)}
									</p>
								</div>
								<Button onClick={createNew} className="gap-1.5">
									<Plus className="h-4 w-4" />
									{t("createManifest", "Create Manifest")}
								</Button>
							</CardContent>
						</Card>
					</motion.div>
				)}
			</AnimatePresence>
		</div>
	);
}
