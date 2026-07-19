"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
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
	Switch,
	Textarea,
} from "@flow-like/flow-like-ui";
import { AnimatePresence, motion } from "framer-motion";
import {
	AlertCircle,
	ArrowLeft,
	FileText,
	FolderOpen,
	Globe,
	HardDrive,
	Loader2,
	Lock,
	Package,
	Plus,
	Save,
	Trash2,
	Zap,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

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
	permissions: {
		memory: string;
		timeout: string;
		network: {
			http_enabled: boolean;
			allowed_hosts: string[];
			websocket_enabled: boolean;
			tcp_enabled: boolean;
			udp_enabled: boolean;
			dns_enabled: boolean;
		};
		filesystem: {
			node_storage: boolean;
			user_storage: boolean;
			upload_dir: boolean;
			cache_dir: boolean;
		};
		variables: boolean;
		cache: boolean;
		streaming: boolean;
		a2ui: boolean;
		models: boolean;
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
		name: "My Package",
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
				http_enabled: false,
				allowed_hosts: [],
				websocket_enabled: false,
				tcp_enabled: false,
				udp_enabled: false,
				dns_enabled: false,
			},
			filesystem: {
				node_storage: false,
				user_storage: false,
				upload_dir: false,
				cache_dir: false,
			},
			variables: false,
			cache: false,
			streaming: false,
			a2ui: false,
			models: false,
			oauth_scopes: [],
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
}: {
	data: ManifestData;
	onChange: (d: ManifestData) => void;
}) {
	return (
		<Card>
			<CardHeader className="pb-3">
				<SectionHeader
					icon={Package}
					title="Package Identity"
					description="Core metadata for your package"
				/>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid grid-cols-2 gap-4">
					<div className="space-y-1.5">
						<Label className="text-xs">Package ID</Label>
						<Input
							value={data.id}
							onChange={(e) => onChange({ ...data, id: e.target.value })}
							placeholder="com.example.my-package"
							className="h-9 font-mono text-xs"
						/>
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs">Version</Label>
						<Input
							value={data.version}
							onChange={(e) => onChange({ ...data, version: e.target.value })}
							placeholder="0.1.0"
							className="h-9 font-mono text-xs"
						/>
					</div>
				</div>
				<div className="grid grid-cols-2 gap-4">
					<div className="space-y-1.5">
						<Label className="text-xs">Name</Label>
						<Input
							value={data.name}
							onChange={(e) => onChange({ ...data, name: e.target.value })}
							className="h-9"
						/>
					</div>
					<div className="space-y-1.5">
						<Label className="text-xs">License</Label>
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
					<Label className="text-xs">Description</Label>
					<Textarea
						value={data.description}
						onChange={(e) => onChange({ ...data, description: e.target.value })}
						rows={2}
						className="text-sm"
						placeholder="What does this package do?"
					/>
				</div>
				<div className="grid grid-cols-2 gap-4">
					<div className="space-y-1.5">
						<Label className="text-xs">Repository</Label>
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
						<Label className="text-xs">Homepage</Label>
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
					<Label className="text-xs">Keywords (comma-separated)</Label>
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
						placeholder="ai, transform, data"
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
				<Label className="text-xs">Authors</Label>
				<Button
					variant="ghost"
					size="sm"
					onClick={addAuthor}
					className="h-7 text-xs"
				>
					<Plus className="h-3 w-3 mr-1" />
					Add
				</Button>
			</div>
			{authors.map((author, i) => (
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

function PermissionsSection({
	data,
	onChange,
}: {
	data: ManifestData;
	onChange: (d: ManifestData) => void;
}) {
	const p = data.permissions ?? {};
	const net = p.network ?? {
		http_enabled: false,
		allowed_hosts: [],
		websocket_enabled: false,
		tcp_enabled: false,
		udp_enabled: false,
		dns_enabled: false,
	};
	const fs = p.filesystem ?? {
		node_storage: false,
		user_storage: false,
		upload_dir: false,
		cache_dir: false,
	};
	const updatePerm = (patch: Partial<ManifestData["permissions"]>) =>
		onChange({
			...data,
			permissions: { ...p, network: net, filesystem: fs, ...patch },
		});

	return (
		<Card>
			<CardHeader className="pb-3">
				<SectionHeader
					icon={Lock}
					title="Permissions"
					description="Declare what your package needs access to"
				/>
			</CardHeader>
			<CardContent className="space-y-5">
				{/* Resources */}
				<div className="space-y-3">
					<div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
						<HardDrive className="h-3 w-3" /> Resources
					</div>
					<div className="grid grid-cols-2 gap-4">
						<div className="space-y-1.5">
							<Label className="text-xs">Memory</Label>
							<Select
								value={p.memory}
								onValueChange={(v) => updatePerm({ memory: v })}
							>
								<SelectTrigger className="h-9">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{MEMORY_TIERS.map((t) => (
										<SelectItem key={t.value} value={t.value}>
											{t.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-1.5">
							<Label className="text-xs">Timeout</Label>
							<Select
								value={p.timeout}
								onValueChange={(v) => updatePerm({ timeout: v })}
							>
								<SelectTrigger className="h-9">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{TIMEOUT_TIERS.map((t) => (
										<SelectItem key={t.value} value={t.value}>
											{t.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
					</div>
				</div>

				<Separator />

				{/* Network */}
				<div className="space-y-3">
					<div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
						<Globe className="h-3 w-3" /> Network
					</div>
					<div className="space-y-3">
						<PermToggle
							label="HTTP Access"
							checked={net.http_enabled}
							onChange={(v) =>
								updatePerm({
									network: { ...net, http_enabled: v },
								})
							}
						/>
						{net.http_enabled && (
							<motion.div
								initial={{ opacity: 0, height: 0 }}
								animate={{ opacity: 1, height: "auto" }}
								exit={{ opacity: 0, height: 0 }}
								className="space-y-1.5 pl-6"
							>
								<Label className="text-xs">
									Allowed Hosts (comma-separated, empty = all)
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
									placeholder="api.example.com, cdn.example.com"
									className="h-8 text-xs"
								/>
							</motion.div>
						)}
						<PermToggle
							label="WebSocket"
							checked={net.websocket_enabled}
							onChange={(v) =>
								updatePerm({
									network: {
										...net,
										websocket_enabled: v,
									},
								})
							}
						/>
						<PermToggle
							label="TCP Sockets"
							checked={net.tcp_enabled ?? false}
							onChange={(v) =>
								updatePerm({
									network: { ...net, tcp_enabled: v },
								})
							}
						/>
						<PermToggle
							label="UDP Sockets"
							checked={net.udp_enabled ?? false}
							onChange={(v) =>
								updatePerm({
									network: { ...net, udp_enabled: v },
								})
							}
						/>
						<PermToggle
							label="DNS Lookups"
							checked={net.dns_enabled ?? false}
							onChange={(v) =>
								updatePerm({
									network: { ...net, dns_enabled: v },
								})
							}
						/>
					</div>
				</div>

				<Separator />

				{/* Filesystem */}
				<div className="space-y-3">
					<div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
						<HardDrive className="h-3 w-3" /> Filesystem
					</div>
					<div className="space-y-2">
						<PermToggle
							label="Node Storage"
							checked={fs.node_storage}
							onChange={(v) =>
								updatePerm({
									filesystem: {
										...fs,
										node_storage: v,
									},
								})
							}
						/>
						<PermToggle
							label="User Storage"
							checked={fs.user_storage}
							onChange={(v) =>
								updatePerm({
									filesystem: {
										...fs,
										user_storage: v,
									},
								})
							}
						/>
						<PermToggle
							label="Upload Directory"
							checked={fs.upload_dir}
							onChange={(v) =>
								updatePerm({
									filesystem: {
										...fs,
										upload_dir: v,
									},
								})
							}
						/>
						<PermToggle
							label="Cache Directory"
							checked={fs.cache_dir}
							onChange={(v) =>
								updatePerm({
									filesystem: {
										...fs,
										cache_dir: v,
									},
								})
							}
						/>
					</div>
				</div>

				<Separator />

				{/* Capabilities */}
				<div className="space-y-3">
					<div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
						<Zap className="h-3 w-3" /> Capabilities
					</div>
					<div className="space-y-2">
						<PermToggle
							label="Variables"
							checked={p.variables}
							onChange={(v) => updatePerm({ variables: v })}
						/>
						<PermToggle
							label="Cache"
							checked={p.cache}
							onChange={(v) => updatePerm({ cache: v })}
						/>
						<PermToggle
							label="Streaming"
							checked={p.streaming}
							onChange={(v) => updatePerm({ streaming: v })}
						/>
						<PermToggle
							label="A2UI"
							checked={p.a2ui}
							onChange={(v) => updatePerm({ a2ui: v })}
						/>
						<PermToggle
							label="Models / LLM"
							checked={p.models}
							onChange={(v) => updatePerm({ models: v })}
						/>
					</div>
				</div>
			</CardContent>
		</Card>
	);
}

function PermToggle({
	label,
	checked,
	onChange,
}: {
	label: string;
	checked: boolean;
	onChange: (v: boolean) => void;
}) {
	return (
		<div className="flex items-center justify-between">
			<Label className="text-xs">{label}</Label>
			<Switch checked={checked} onCheckedChange={onChange} />
		</div>
	);
}

function ManifestEditorContent() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const initialPath = searchParams.get("path") ?? "";
	const [projectPath, setProjectPath] = useState(initialPath);
	const [data, setData] = useState<ManifestData | null>(null);
	const [loading, setLoading] = useState(false);
	const [saving, setSaving] = useState(false);
	const [hasChanges, setHasChanges] = useState(false);

	const loadManifest = useCallback(async (path: string) => {
		if (!path) return;
		setLoading(true);
		try {
			const raw = await invoke<Record<string, unknown>>(
				"developer_get_manifest",
				{ projectPath: path },
			);
			setData(raw as unknown as ManifestData);
			setHasChanges(false);
		} catch {
			setData(null);
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		if (initialPath) loadManifest(initialPath);
	}, [initialPath, loadManifest]);

	const selectProject = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (!selected) return;
		setProjectPath(selected);
		await loadManifest(selected);
	}, [loadManifest]);

	const createNew = useCallback(() => {
		setData(createDefaultManifest());
		setHasChanges(true);
	}, []);

	const handleChange = useCallback((updated: ManifestData) => {
		setData(updated);
		setHasChanges(true);
	}, []);

	const saveManifest = useCallback(async () => {
		if (!data || !projectPath) {
			if (!projectPath) {
				const selected = await open({ directory: true, multiple: false });
				if (!selected) return;
				setProjectPath(selected);
				setSaving(true);
				try {
					await invoke("developer_save_manifest", {
						projectPath: selected,
						manifest: data,
					});
					toast.success("Manifest saved");
					setHasChanges(false);
				} catch (err) {
					toast.error(`Failed to save: ${err}`);
				} finally {
					setSaving(false);
				}
				return;
			}
			return;
		}

		setSaving(true);
		try {
			await invoke("developer_save_manifest", {
				projectPath,
				manifest: data,
			});
			toast.success("Manifest saved");
			setHasChanges(false);
		} catch (err) {
			toast.error(`Failed to save: ${err}`);
		} finally {
			setSaving(false);
		}
	}, [data, projectPath]);

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between pb-4 border-b">
				<div className="flex items-center gap-4">
					<Button
						variant="ghost"
						size="icon"
						className="rounded-full"
						onClick={() => router.push("/developer")}
					>
						<ArrowLeft className="h-5 w-5" />
					</Button>
					<div>
						<div className="flex items-center gap-2">
							<FileText className="h-5 w-5 text-primary" />
							<h1 className="text-2xl font-bold">Manifest Editor</h1>
						</div>
						<p className="text-sm text-muted-foreground">
							Configure your flow-like.toml visually
						</p>
					</div>
				</div>
				<div className="flex items-center gap-2">
					{hasChanges && (
						<Badge
							variant="outline"
							className="text-xs text-amber-600 border-amber-500/30"
						>
							Unsaved changes
						</Badge>
					)}
					<Button
						onClick={saveManifest}
						disabled={saving || !data}
						size="sm"
						className="gap-1.5"
					>
						{saving ? (
							<Loader2 className="h-4 w-4 animate-spin" />
						) : (
							<Save className="h-4 w-4" />
						)}
						Save
					</Button>
				</div>
			</div>

			<div className="flex-1 overflow-y-auto py-4 space-y-4">
				{/* Project Selector */}
				<Card>
					<CardContent className="p-4">
						<div className="flex items-center gap-3">
							<FolderOpen className="h-5 w-5 text-muted-foreground shrink-0" />
							<Input
								value={projectPath}
								onChange={(e) => setProjectPath(e.target.value)}
								placeholder="Select a project directory..."
								className="flex-1 h-9"
								readOnly
							/>
							<Button variant="outline" size="sm" onClick={selectProject}>
								<FolderOpen className="h-4 w-4 mr-2" />
								Open
							</Button>
							<Button
								variant="outline"
								size="sm"
								onClick={() => {
									if (projectPath) loadManifest(projectPath);
								}}
								disabled={!projectPath || loading}
							>
								{loading ? (
									<Loader2 className="h-4 w-4 animate-spin" />
								) : (
									"Load"
								)}
							</Button>
							<Button variant="outline" size="sm" onClick={createNew}>
								<Plus className="h-4 w-4 mr-2" />
								New
							</Button>
						</div>
					</CardContent>
				</Card>

				<AnimatePresence mode="wait">
					{data && (
						<motion.div
							key="editor"
							initial={{ opacity: 0, y: 10 }}
							animate={{ opacity: 1, y: 0 }}
							exit={{ opacity: 0, y: -10 }}
							className="space-y-4"
						>
							<IdentitySection data={data} onChange={handleChange} />
							<PermissionsSection data={data} onChange={handleChange} />
						</motion.div>
					)}

					{!data && !loading && projectPath && (
						<motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
							<Card>
								<CardContent className="py-12 text-center space-y-3">
									<AlertCircle className="h-10 w-10 text-muted-foreground mx-auto" />
									<div>
										<p className="font-medium">No flow-like.toml found</p>
										<p className="text-sm text-muted-foreground">
											Create a new manifest for this project
										</p>
									</div>
									<Button onClick={createNew} className="gap-1.5">
										<Plus className="h-4 w-4" />
										Create Manifest
									</Button>
								</CardContent>
							</Card>
						</motion.div>
					)}

					{!data && !loading && !projectPath && (
						<Card>
							<CardContent className="py-16 text-center space-y-3">
								<FileText className="h-12 w-12 text-muted-foreground/30 mx-auto" />
								<div>
									<p className="font-medium">
										Select a project to edit its manifest
									</p>
									<p className="text-sm text-muted-foreground">
										Or create a new one from scratch
									</p>
								</div>
							</CardContent>
						</Card>
					)}
				</AnimatePresence>
			</div>
		</div>
	);
}

export default function ManifestPage() {
	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full">
					<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
				</div>
			}
		>
			<ManifestEditorContent />
		</Suspense>
	);
}
