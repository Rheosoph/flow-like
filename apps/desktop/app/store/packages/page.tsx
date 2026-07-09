"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
	Badge,
	Button,
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	EmptyState,
	ExploreHubHeader,
	Input,
	type InstalledPackage,
	Label,
	PackageDetailWrapper,
	PackageListContent,
	PackageStatusBadge,
	type PackageUpdate,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@flow-like/flow-like-ui";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
} from "@flow-like/flow-like-ui/components/ui/avatar";
import {
	hashToGradient,
	useThemeInfo,
} from "@flow-like/flow-like-ui/hooks/use-theme-gradient";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import type {
	DeveloperProject,
	DeveloperSettings,
	PackageInspection,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import {
	EDITOR_OPTIONS,
	TEMPLATE_LANGUAGES,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import {
	AlertCircle,
	AlertTriangle,
	Bug,
	CheckCircle,
	CloudUpload,
	Code2,
	Download,
	ExternalLink,
	FolderOpen,
	Globe,
	Loader2,
	Lock,
	Package,
	Pencil,
	Plus,
	RefreshCw,
	Search,
	Settings2,
	Sparkles,
	Trash2,
	Upload,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import {
	Suspense,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	usePackageStatus,
	usePackageStatusMap,
} from "../../../hooks/use-package-status";
import { fetcher } from "../../../lib/api";
import { countBySeverity, lintNodes } from "../../../lib/validate-nodes";

// ─── Shared Helpers ──────────────────────────────────────────────────────────

function useIsVisible(ref: React.RefObject<HTMLElement | null>) {
	const [isVisible, setIsVisible] = useState(false);
	useEffect(() => {
		const el = ref.current;
		if (!el) return;
		const observer = new IntersectionObserver(
			([entry]) => {
				if (entry?.isIntersecting) {
					setIsVisible(true);
					observer.disconnect();
				}
			},
			{ rootMargin: "200px" },
		);
		observer.observe(el);
		return () => observer.disconnect();
	}, [ref]);
	return isVisible;
}

function LanguageBadge({ language }: { language: string }) {
	const info = TEMPLATE_LANGUAGES.find((t) => t.value === language);
	return (
		<Badge
			variant="secondary"
			className="gap-1.5 text-[10px] whitespace-nowrap rounded-full px-2 py-0.5 bg-muted/30 border-transparent text-foreground"
		>
			{info?.img ? (
				<img
					src={info.img}
					alt={info.label}
					className="w-5 h-5 rounded-full object-cover"
				/>
			) : (
				<span>{info?.icon ?? "📦"}</span>
			)}
			{info?.label ?? language}
		</Badge>
	);
}

// ─── My Projects Tab ─────────────────────────────────────────────────────────

function ProjectCard({
	project,
	onRemove,
}: {
	project: DeveloperProject;
	onRemove: (id: string) => void;
}) {
	const [opening, setOpening] = useState(false);
	const [loading, setLoading] = useState(false);
	const [inspection, setInspection] = useState<PackageInspection | null>(null);
	const [inspecting, setInspecting] = useState(false);
	const compileStatus = usePackageStatus(`dev:${project.path}`);
	const cardRef = useRef<HTMLDivElement>(null);
	const isVisible = useIsVisible(cardRef);
	const { primaryHue, isDark } = useThemeInfo();
	const gradient = useMemo(
		() => hashToGradient(project.id, primaryHue, isDark),
		[project.id, primaryHue, isDark],
	);

	useEffect(() => {
		if (!isVisible || inspection || inspecting) return;
		let cancelled = false;
		setInspecting(true);
		invoke<PackageInspection>("developer_inspect_package", {
			projectPath: project.path,
		})
			.then((result) => {
				if (!cancelled) setInspection(result);
			})
			.catch(() => {})
			.finally(() => {
				if (!cancelled) setInspecting(false);
			});
		return () => {
			cancelled = true;
		};
	}, [isVisible, project.path, inspection, inspecting]);

	const openInEditor = async () => {
		setOpening(true);
		try {
			await invoke("developer_open_in_editor", {
				projectPath: project.path,
			});
		} catch (err) {
			toast.error(getErrorMessage(err));
		} finally {
			setOpening(false);
		}
	};

	const loadIntoCatalog = async () => {
		setLoading(true);
		try {
			const count = await invoke<number>("developer_load_into_catalog", {
				projectPath: project.path,
			});
			toast.success(`Loaded ${count} node(s) into catalog`);
		} catch (err) {
			toast.error(getErrorMessage(err));
		} finally {
			setLoading(false);
		}
	};

	const nodeCount = inspection?.nodes?.length ?? 0;
	const lintCounts = useMemo(
		() =>
			inspection?.nodes ? countBySeverity(lintNodes(inspection.nodes)) : null,
		[inspection?.nodes],
	);

	return (
		<div
			ref={cardRef}
			className="group relative flex flex-row rounded-lg border border-border/40 border-dashed bg-card/60 backdrop-blur-sm overflow-hidden transition-all hover:border-primary/40 hover:bg-card/80 hover:shadow-lg"
		>
			{/* Left gradient accent */}
			<div className="relative w-28 shrink-0 overflow-hidden">
				<div
					className="absolute inset-0"
					style={{
						background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
						opacity: gradient.opacity,
					}}
				/>
				<div className="absolute inset-0 bg-linear-to-r from-transparent to-card/80" />
				<div className="absolute inset-0 flex items-center justify-center">
					<Avatar className="w-10 h-10 rounded-lg shadow-md border-2 border-background/20 bg-background/30 backdrop-blur-sm">
						<AvatarFallback className="rounded-lg text-xs font-mono font-bold bg-background/20 text-white/80">
							<Package className="h-5 w-5" />
						</AvatarFallback>
					</Avatar>
				</div>
			</div>

			{/* Content */}
			<div className="flex-1 min-w-0 px-3.5 py-3 flex flex-col gap-1">
				{/* Top row: name + badges */}
				<div className="flex items-center gap-1.5 min-w-0">
					<h3 className="text-sm font-semibold font-mono truncate group-hover:text-primary transition-colors">
						{project.name}
					</h3>
					{inspection?.manifest?.version && (
						<span className="text-[10px] font-mono text-muted-foreground/50 shrink-0">
							v{inspection.manifest.version}
						</span>
					)}
					{compileStatus && compileStatus !== "idle" && (
						<PackageStatusBadge status={compileStatus} />
					)}
					{lintCounts && lintCounts.errors > 0 && (
						<Tooltip>
							<TooltipTrigger>
								<Badge
									variant="destructive"
									className="text-[10px] px-1.5 py-0 h-4 gap-1"
								>
									<AlertCircle className="h-2.5 w-2.5" />
									{lintCounts.errors}
								</Badge>
							</TooltipTrigger>
							<TooltipContent>
								{lintCounts.errors} lint error
								{lintCounts.errors !== 1 ? "s" : ""}
							</TooltipContent>
						</Tooltip>
					)}
					{lintCounts && lintCounts.errors === 0 && lintCounts.warnings > 0 && (
						<Tooltip>
							<TooltipTrigger>
								<Badge className="text-[10px] px-1.5 py-0 h-4 gap-1 bg-amber-500/10 text-amber-600 border-amber-500/20 hover:bg-amber-500/20">
									<AlertTriangle className="h-2.5 w-2.5" />
									{lintCounts.warnings}
								</Badge>
							</TooltipTrigger>
							<TooltipContent>
								{lintCounts.warnings} lint warning
								{lintCounts.warnings !== 1 ? "s" : ""}
							</TooltipContent>
						</Tooltip>
					)}
					<div className="flex items-center gap-1 ml-auto shrink-0">
						<span className="inline-flex items-center gap-1 rounded bg-background/80 border border-border/40 px-1.5 py-0.5 text-[10px] text-muted-foreground font-mono">
							<Lock className="h-2.5 w-2.5" /> local
						</span>
						<LanguageBadge language={project.language} />
					</div>
				</div>

				{/* Description / path */}
				<p className="text-xs text-muted-foreground/80 line-clamp-1 leading-relaxed">
					{inspection?.manifest?.description || project.path}
				</p>

				{/* Action buttons */}
				<div className="flex items-center gap-0.5 mt-auto pt-1.5 border-t border-border/20">
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								size="icon"
								variant="ghost"
								className="h-6 w-6 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								onClick={openInEditor}
								disabled={opening}
							>
								{opening ? (
									<Loader2 className="h-3 w-3 animate-spin" />
								) : (
									<ExternalLink className="h-3 w-3" />
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent>Open in Editor</TooltipContent>
					</Tooltip>

					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								size="icon"
								variant="ghost"
								className={`h-6 w-6 rounded-full ${
									compileStatus === "stale"
										? "text-orange-600 hover:text-orange-700 hover:bg-orange-500/10 animate-pulse"
										: "text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								}`}
								onClick={loadIntoCatalog}
								disabled={loading}
							>
								{loading ? (
									<Loader2 className="h-3 w-3 animate-spin" />
								) : compileStatus === "stale" ? (
									<RefreshCw className="h-3 w-3" />
								) : (
									<Upload className="h-3 w-3" />
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent>
							{compileStatus === "stale"
								? "WASM changed — Reload into Catalog"
								: "Load into Catalog"}
						</TooltipContent>
					</Tooltip>

					<Tooltip>
						<TooltipTrigger asChild>
							<Link
								href={`/developer/publish?project=${encodeURIComponent(project.path)}`}
							>
								<Button
									size="icon"
									variant="ghost"
									className="h-6 w-6 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								>
									<CloudUpload className="h-3 w-3" />
								</Button>
							</Link>
						</TooltipTrigger>
						<TooltipContent>Publish to Registry</TooltipContent>
					</Tooltip>

					<Tooltip>
						<TooltipTrigger asChild>
							<Link
								href={`/developer/manifest?path=${encodeURIComponent(project.path)}`}
							>
								<Button
									size="icon"
									variant="ghost"
									className="h-6 w-6 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								>
									<Pencil className="h-3 w-3" />
								</Button>
							</Link>
						</TooltipTrigger>
						<TooltipContent>Edit Manifest</TooltipContent>
					</Tooltip>

					<Tooltip>
						<TooltipTrigger asChild>
							<Link
								href={`/developer/debug?project=${encodeURIComponent(project.path)}`}
							>
								<Button
									size="icon"
									variant="ghost"
									className="h-6 w-6 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								>
									<Bug className="h-3 w-3" />
								</Button>
							</Link>
						</TooltipTrigger>
						<TooltipContent>Debug &amp; Test</TooltipContent>
					</Tooltip>

					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								size="icon"
								variant="ghost"
								className="h-6 w-6 rounded-full text-destructive hover:text-destructive hover:bg-destructive/10"
								onClick={() => onRemove(project.id)}
							>
								<Trash2 className="h-3 w-3" />
							</Button>
						</TooltipTrigger>
						<TooltipContent>Remove</TooltipContent>
					</Tooltip>
				</div>
			</div>
		</div>
	);
}

function SettingsDialog() {
	const [settings, setSettings] = useState<DeveloperSettings | null>(null);
	const [saving, setSaving] = useState(false);

	useEffect(() => {
		invoke<DeveloperSettings>("developer_get_settings")
			.then(setSettings)
			.catch(console.error);
	}, []);

	const handleSave = async () => {
		if (!settings) return;
		setSaving(true);
		try {
			await invoke("developer_save_settings", { devSettings: settings });
			toast.success("Settings saved");
		} catch (err) {
			toast.error(getErrorMessage(err));
		} finally {
			setSaving(false);
		}
	};

	if (!settings) return null;

	return (
		<div className="space-y-4">
			<div className="space-y-2">
				<Label>Preferred Editor</Label>
				<Select
					value={settings.preferredEditor}
					onValueChange={(v) =>
						setSettings({ ...settings, preferredEditor: v })
					}
				>
					<SelectTrigger>
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{EDITOR_OPTIONS.map((e) => (
							<SelectItem key={e.value} value={e.value}>
								{e.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>
			<DialogFooter>
				<DialogClose asChild>
					<Button variant="outline">Cancel</Button>
				</DialogClose>
				<Button onClick={handleSave} disabled={saving}>
					{saving && <Loader2 className="h-4 w-4 animate-spin mr-1" />}
					Save
				</Button>
			</DialogFooter>
		</div>
	);
}

function MyProjectsContent() {
	const [projects, setProjects] = useState<DeveloperProject[]>([]);
	const [isLoading, setIsLoading] = useState(true);
	const [isAdding, setIsAdding] = useState(false);
	const [search, setSearch] = useState("");

	const fetchProjects = useCallback(async () => {
		setIsLoading(true);
		try {
			const list = await invoke<DeveloperProject[]>("developer_list_projects");
			setProjects(list);
		} catch (err) {
			console.error("Failed to list projects:", err);
		} finally {
			setIsLoading(false);
		}
	}, []);

	useEffect(() => {
		fetchProjects();
	}, [fetchProjects]);

	useEffect(() => {
		const checkStaleness = () => {
			invoke("developer_check_staleness").catch(() => {});
		};
		checkStaleness();
		const interval = setInterval(checkStaleness, 5000);
		return () => clearInterval(interval);
	}, []);

	const filtered = useMemo(() => {
		if (!search.trim()) return projects;
		const q = search.toLowerCase();
		return projects.filter(
			(p) =>
				p.name.toLowerCase().includes(q) ||
				p.path.toLowerCase().includes(q) ||
				p.language.toLowerCase().includes(q),
		);
	}, [projects, search]);

	const handleAddExisting = async () => {
		try {
			const selected = await open({ directory: true, multiple: false });
			if (!selected) return;

			setIsAdding(true);

			let projectName = selected.split("/").pop() ?? "Untitled";
			try {
				const manifest = await invoke<Record<string, unknown>>(
					"developer_get_manifest",
					{ projectPath: selected },
				);
				const pkg = manifest.package as Record<string, unknown>;
				if (pkg?.name) projectName = pkg.name as string;
			} catch {
				// no manifest
			}

			let detectedLang = "rust";
			const { exists } = await import("@tauri-apps/plugin-fs");
			const detectionMap: [string, string][] = [
				["Cargo.toml", "rust"],
				["package.json", "typescript"],
				["tsconfig.json", "typescript"],
				["go.mod", "go"],
				["build.zig", "zig"],
				["moon.mod.json", "moonbit"],
				["nimble.nimble", "nim"],
				[".csproj", "csharp"],
				["build.gradle.kts", "kotlin"],
				["CMakeLists.txt", "cpp"],
				["requirements.txt", "python"],
				["pyproject.toml", "python"],
			];
			for (const [file, lang] of detectionMap) {
				try {
					if (await exists(`${selected}/${file}`)) {
						detectedLang = lang;
						break;
					}
				} catch {
					// skip
				}
			}

			await invoke("developer_add_project", {
				input: { path: selected, language: detectedLang, name: projectName },
			});
			toast.success(`Added ${projectName}`);
			await fetchProjects();
		} catch (err) {
			toast.error(getErrorMessage(err));
		} finally {
			setIsAdding(false);
		}
	};

	const handleRemove = async (id: string) => {
		try {
			await invoke("developer_remove_project", { projectId: id });
			toast.success("Project removed");
			await fetchProjects();
		} catch (err) {
			toast.error(getErrorMessage(err));
		}
	};

	return (
		<div className="space-y-4">
			<div className="flex items-center gap-3">
				<div className="relative flex-1">
					<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/40" />
					<Input
						placeholder="Search projects…"
						value={search}
						onChange={(e) => setSearch(e.target.value)}
						className="pl-10 rounded-full bg-muted/30 border-border/20"
					/>
				</div>

				<Dialog>
					<Tooltip>
						<TooltipTrigger asChild>
							<DialogTrigger asChild>
								<Button
									size="icon"
									variant="ghost"
									className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
								>
									<Settings2 className="h-4 w-4" />
								</Button>
							</DialogTrigger>
						</TooltipTrigger>
						<TooltipContent>Settings</TooltipContent>
					</Tooltip>
					<DialogContent className="max-w-sm">
						<DialogHeader>
							<DialogTitle>Developer Settings</DialogTitle>
							<DialogDescription>
								Configure your development environment
							</DialogDescription>
						</DialogHeader>
						<SettingsDialog />
					</DialogContent>
				</Dialog>

				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							size="icon"
							variant="ghost"
							className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
							onClick={handleAddExisting}
							disabled={isAdding}
						>
							{isAdding ? (
								<Loader2 className="h-4 w-4 animate-spin" />
							) : (
								<FolderOpen className="h-4 w-4" />
							)}
						</Button>
					</TooltipTrigger>
					<TooltipContent>Add Existing</TooltipContent>
				</Tooltip>

				<Tooltip>
					<TooltipTrigger asChild>
						<Link href="/developer/new">
							<Button
								size="icon"
								variant="ghost"
								className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
							>
								<Plus className="h-4 w-4" />
							</Button>
						</Link>
					</TooltipTrigger>
					<TooltipContent>New Project</TooltipContent>
				</Tooltip>

				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							size="icon"
							variant="ghost"
							className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
							onClick={fetchProjects}
							disabled={isLoading}
						>
							<RefreshCw
								className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`}
							/>
						</Button>
					</TooltipTrigger>
					<TooltipContent>Refresh</TooltipContent>
				</Tooltip>
			</div>

			{isLoading ? (
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
					{Array.from({ length: 6 }).map((_, i) => (
						<div
							key={`skel-${i}`}
							className="flex flex-col rounded-lg border border-border/40 border-dashed bg-card/60 overflow-hidden"
						>
							<Skeleton className="h-20 w-full rounded-none" />
							<div className="px-3.5 pt-5 pb-3 space-y-2">
								<Skeleton className="h-4 w-28 rounded" />
								<Skeleton className="h-3 w-full rounded" />
								<Skeleton className="h-3 w-3/4 rounded" />
							</div>
						</div>
					))}
				</div>
			) : filtered.length > 0 ? (
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
					{filtered.map((p) => (
						<ProjectCard key={p.id} project={p} onRemove={handleRemove} />
					))}
				</div>
			) : projects.length > 0 ? (
				<div className="flex flex-col items-center justify-center py-20">
					<p className="text-sm text-muted-foreground/50">
						No projects match &quot;{search}&quot;
					</p>
				</div>
			) : (
				<EmptyState
					icons={[Code2, Sparkles, Package]}
					title="No node projects yet"
					description="Create a new node project from a template, or add an existing one from disk."
					action={[
						{
							label: "New Project",
							onClick: () => {
								window.location.href = "/developer/new";
							},
						},
						{
							label: "Add Existing",
							onClick: handleAddExisting,
						},
					]}
					className="border border-dashed border-border/30 rounded-2xl bg-muted/5"
				/>
			)}
		</div>
	);
}

// ─── Installed Tab ───────────────────────────────────────────────────────────

function InstalledPackageCard({
	pkg,
	updateAvailable,
	onUninstall,
	onUpdate,
	isUpdating,
	isUninstalling,
	compileStatus,
}: {
	pkg: InstalledPackage;
	updateAvailable?: string;
	onUninstall: () => void;
	onUpdate: () => void;
	isUpdating: boolean;
	isUninstalling: boolean;
	compileStatus?:
		| "idle"
		| "downloading"
		| "compiling"
		| "ready"
		| "error"
		| "stale";
}) {
	const { primaryHue, isDark } = useThemeInfo();
	const gradient = useMemo(
		() => hashToGradient(pkg.id, primaryHue, isDark),
		[pkg.id, primaryHue, isDark],
	);
	const displayName = pkg.metadata?.name ?? pkg.manifest.name;
	const displayDesc = pkg.metadata?.description ?? pkg.manifest.description;
	const icon = pkg.metadata?.icon;
	const thumbnail = pkg.metadata?.thumbnail;

	return (
		<Link
			href={`/store/packages?id=${pkg.id}`}
			className="group relative flex flex-row rounded-lg border border-border/40 border-dashed bg-card/60 backdrop-blur-sm overflow-hidden transition-all hover:border-primary/40 hover:bg-card/80 hover:shadow-lg cursor-pointer"
		>
			{/* Left gradient accent */}
			<div className="relative w-28 shrink-0 overflow-hidden">
				{thumbnail ? (
					<img
						src={thumbnail}
						alt=""
						className="absolute inset-0 w-full h-full object-cover"
					/>
				) : (
					<div
						className="absolute inset-0"
						style={{
							background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
							opacity: gradient.opacity,
						}}
					/>
				)}
				<div className="absolute inset-0 bg-linear-to-r from-transparent to-card/80" />
				<div className="absolute inset-0 flex items-center justify-center">
					<Avatar className="w-10 h-10 rounded-lg shadow-md border-2 border-background/20 bg-background/30 backdrop-blur-sm">
						{icon ? (
							<AvatarImage
								src={icon}
								alt={displayName}
								className="rounded-lg"
							/>
						) : null}
						<AvatarFallback className="rounded-lg text-xs font-mono font-bold bg-background/20 text-white/80">
							<Package className="h-5 w-5" />
						</AvatarFallback>
					</Avatar>
				</div>
			</div>

			{/* Content */}
			<div className="flex-1 min-w-0 px-3.5 py-3 flex flex-col gap-1">
				{/* Top row: name + badges */}
				<div className="flex items-center gap-1.5 min-w-0">
					<h3 className="text-sm font-semibold font-mono truncate group-hover:text-primary transition-colors">
						{displayName}
					</h3>
					<span className="text-[10px] font-mono text-muted-foreground/50 shrink-0">
						v{pkg.version}
						{updateAvailable && (
							<span className="text-primary"> → v{updateAvailable}</span>
						)}
					</span>
					<div className="flex items-center gap-1 ml-auto shrink-0">
						<span className="inline-flex items-center gap-1 rounded bg-background/80 border border-border/40 px-1.5 py-0.5 text-[10px] text-muted-foreground font-mono">
							<Globe className="h-2.5 w-2.5" /> installed
						</span>
						{compileStatus && compileStatus !== "idle" && (
							<PackageStatusBadge status={compileStatus} />
						)}
						{updateAvailable && (
							<Badge
								variant="secondary"
								className="gap-0.5 text-[10px] px-1.5 py-0.5"
							>
								<AlertCircle className="h-2.5 w-2.5" />
								Update
							</Badge>
						)}
					</div>
				</div>

				{/* Description */}
				<p className="text-xs text-muted-foreground/80 line-clamp-1 leading-relaxed">
					{displayDesc}
				</p>

				{/* Footer with keywords + actions */}
				<div className="flex items-center gap-1 mt-auto pt-1.5 border-t border-border/20">
					{pkg.manifest.keywords.length > 0 && (
						<div className="flex items-center gap-1 flex-1 min-w-0 overflow-hidden">
							{pkg.manifest.keywords.slice(0, 2).map((kw) => (
								<span
									key={kw}
									className="rounded bg-muted/30 px-1.5 py-0.5 text-[10px] text-muted-foreground/60 font-mono truncate"
								>
									{kw}
								</span>
							))}
						</div>
					)}
					<div className="flex items-center gap-0.5 shrink-0 ml-auto">
						{updateAvailable && (
							<Button
								size="icon"
								variant="ghost"
								className="h-6 w-6 rounded-full text-primary hover:text-primary hover:bg-primary/10"
								onClick={(e) => {
									e.preventDefault();
									e.stopPropagation();
									onUpdate();
								}}
								disabled={isUpdating}
							>
								{isUpdating ? (
									<RefreshCw className="h-3 w-3 animate-spin" />
								) : (
									<RefreshCw className="h-3 w-3" />
								)}
							</Button>
						)}
						<Button
							size="icon"
							variant="ghost"
							className="h-6 w-6 rounded-full text-destructive hover:text-destructive hover:bg-destructive/10"
							onClick={(e) => {
								e.preventDefault();
								e.stopPropagation();
								onUninstall();
							}}
							disabled={isUninstalling}
						>
							{isUninstalling ? (
								<Loader2 className="h-3 w-3 animate-spin" />
							) : (
								<Trash2 className="h-3 w-3" />
							)}
						</Button>
					</div>
				</div>
			</div>
		</Link>
	);
}

function InstalledContent() {
	const queryClient = useQueryClient();
	const auth = useAuth();
	const [searchQuery, setSearchQuery] = useState("");
	const packageStatusMap = usePackageStatusMap();
	const [updatingPackages, setUpdatingPackages] = useState<Set<string>>(
		new Set(),
	);
	const [uninstallingPackages, setUninstallingPackages] = useState<Set<string>>(
		new Set(),
	);

	const registryReady = useQuery({
		queryKey: ["registry-init"],
		queryFn: async () => {
			await invoke("registry_init", { config: null });
			return true;
		},
		staleTime: Number.POSITIVE_INFINITY,
	});

	const installedPackages = useQuery({
		queryKey: ["installed-packages"],
		queryFn: async () => {
			return invoke<InstalledPackage[]>("registry_get_installed_packages");
		},
		enabled: registryReady.data === true,
	});

	const availableUpdates = useQuery({
		queryKey: ["available-updates"],
		queryFn: async () => {
			return invoke<PackageUpdate[]>("registry_check_for_updates", {
				token: auth.user?.access_token,
			});
		},
		enabled: registryReady.data === true,
	});

	const updateMutation = useMutation({
		mutationFn: async ({
			packageId,
			version,
		}: { packageId: string; version?: string }) => {
			setUpdatingPackages((prev) => new Set(prev).add(packageId));
			await invoke("registry_update_package", {
				packageId,
				version,
				token: auth.user?.access_token,
			});
		},
		onSuccess: (
			_: void,
			{ packageId }: { packageId: string; version?: string },
		) => {
			toast.success("Package updated successfully");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (error: unknown) => {
			toast.error(`Failed to update package: ${getErrorMessage(error)}`);
		},
		onSettled: (
			_: void | undefined,
			__: unknown,
			{ packageId }: { packageId: string; version?: string },
		) => {
			setUpdatingPackages((prev) => {
				const next = new Set(prev);
				next.delete(packageId);
				return next;
			});
		},
	});

	const uninstallMutation = useMutation({
		mutationFn: async (packageId: string) => {
			setUninstallingPackages((prev) => new Set(prev).add(packageId));
			await invoke("registry_uninstall_package", { packageId });
		},
		onSuccess: (_: void, packageId: string) => {
			toast.success("Package uninstalled");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
			queryClient.invalidateQueries({
				queryKey: ["installed-package", packageId],
			});
		},
		onError: (error: unknown) => {
			toast.error(`Failed to uninstall package: ${getErrorMessage(error)}`);
		},
		onSettled: (_: void | undefined, __: unknown, packageId: string) => {
			setUninstallingPackages((prev) => {
				const next = new Set(prev);
				next.delete(packageId);
				return next;
			});
		},
	});

	const updateAllMutation = useMutation({
		mutationFn: async () => {
			const updates = availableUpdates.data ?? [];
			for (const update of updates) {
				await invoke("registry_update_package", {
					packageId: update.packageId,
					version: update.latestVersion,
					token: auth.user?.access_token,
				});
			}
		},
		onSuccess: () => {
			toast.success("All packages updated");
			queryClient.invalidateQueries({ queryKey: ["installed-packages"] });
			queryClient.invalidateQueries({ queryKey: ["available-updates"] });
		},
		onError: (error: unknown) => {
			toast.error(`Failed to update packages: ${getErrorMessage(error)}`);
		},
	});

	const updatesMap = new Map(
		(availableUpdates.data ?? []).map((u) => [u.packageId, u.latestVersion]),
	);

	const filteredPackages = (installedPackages.data ?? []).filter((pkg) => {
		if (!searchQuery) return true;
		const query = searchQuery.toLowerCase();
		return (
			pkg.manifest.name.toLowerCase().includes(query) ||
			pkg.manifest.description.toLowerCase().includes(query) ||
			pkg.manifest.keywords.some((kw) => kw.toLowerCase().includes(query))
		);
	});

	const hasUpdates = (availableUpdates.data?.length ?? 0) > 0;

	return (
		<div className="space-y-4">
			<div className="flex items-center gap-3">
				<div className="relative flex-1">
					<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/40" />
					<Input
						placeholder="Search installed packages…"
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						className="pl-10 rounded-full bg-muted/30 border-border/20"
					/>
				</div>
				{hasUpdates && (
					<Button
						size="sm"
						onClick={() => updateAllMutation.mutate()}
						disabled={updateAllMutation.isPending}
						className="gap-1.5 rounded-full"
					>
						{updateAllMutation.isPending ? (
							<RefreshCw className="h-3.5 w-3.5 animate-spin" />
						) : (
							<RefreshCw className="h-3.5 w-3.5" />
						)}
						Update All ({availableUpdates.data?.length})
					</Button>
				)}
			</div>

			{installedPackages.data && (
				<div className="flex gap-4 text-xs text-muted-foreground/60">
					<span className="flex items-center gap-1">
						<CheckCircle className="h-3.5 w-3.5 text-green-500" />
						{installedPackages.data.length} installed
					</span>
					{hasUpdates && (
						<span className="flex items-center gap-1">
							<AlertCircle className="h-3.5 w-3.5 text-yellow-500" />
							{availableUpdates.data?.length} updates
						</span>
					)}
				</div>
			)}

			{registryReady.isLoading ||
			installedPackages.isLoading ||
			(!registryReady.isSuccess && !registryReady.isError) ? (
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
					{Array.from({ length: 6 }).map((_, i) => (
						<div
							key={`skel-${i}`}
							className="flex flex-col rounded-lg border border-border/40 border-dashed bg-card/60 overflow-hidden"
						>
							<Skeleton className="h-20 w-full rounded-none" />
							<div className="px-3.5 pt-5 pb-3 space-y-2">
								<Skeleton className="h-4 w-28 rounded" />
								<Skeleton className="h-3 w-full rounded" />
								<Skeleton className="h-3 w-3/4 rounded" />
							</div>
						</div>
					))}
				</div>
			) : filteredPackages.length === 0 ? (
				<EmptyState
					icons={[Download, Package, Sparkles]}
					title={searchQuery ? "No matching packages" : "No packages installed"}
					description={
						searchQuery
							? "Try a different search term"
							: "Browse the Explore tab to find and install packages"
					}
					className="border border-dashed border-border/30 rounded-2xl bg-muted/5"
				/>
			) : (
				<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
					{filteredPackages.map((pkg) => (
						<InstalledPackageCard
							key={pkg.id}
							pkg={pkg}
							updateAvailable={updatesMap.get(pkg.id)}
							onUninstall={() => uninstallMutation.mutate(pkg.id)}
							onUpdate={() =>
								updateMutation.mutate({
									packageId: pkg.id,
									version: updatesMap.get(pkg.id),
								})
							}
							isUpdating={updatingPackages.has(pkg.id)}
							isUninstalling={uninstallingPackages.has(pkg.id)}
							compileStatus={packageStatusMap.get(pkg.id)}
						/>
					))}
				</div>
			)}
		</div>
	);
}

// ─── Page Shell ──────────────────────────────────────────────────────────────

function PackagesHub() {
	const auth = useAuth();
	const router = useRouter();
	const searchParams = useSearchParams();
	const statusMap = usePackageStatusMap();
	const getPackageStatus = useCallback(
		(packageId: string) => statusMap.get(packageId),
		[statusMap],
	);

	const validTabs = ["explore", "installed", "projects"];
	const tabParam = searchParams.get("tab");
	const tab = validTabs.includes(tabParam ?? "") ? tabParam! : "explore";

	const setTab = useCallback(
		(value: string) => {
			const params = new URLSearchParams(searchParams.toString());
			if (value === "explore") {
				params.delete("tab");
			} else {
				params.set("tab", value);
			}
			const qs = params.toString();
			router.push(qs ? `/store/packages?${qs}` : "/store/packages");
		},
		[router, searchParams],
	);

	return (
		<main className="flex-col flex grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-7xl space-y-6">
				<ExploreHubHeader
					active="packages"
					subtitle="Discover, install, and manage WASM node packages."
				/>

				<Tabs value={tab} onValueChange={setTab}>
					<TabsList>
						<TabsTrigger value="explore">
							<Search className="h-3.5 w-3.5 mr-1.5" />
							Explore
						</TabsTrigger>
						<TabsTrigger value="installed">
							<Download className="h-3.5 w-3.5 mr-1.5" />
							Installed
						</TabsTrigger>
						<TabsTrigger value="projects">
							<Code2 className="h-3.5 w-3.5 mr-1.5" />
							My Projects
						</TabsTrigger>
					</TabsList>

					<TabsContent value="explore" className="mt-6">
						<PackageListContent fetcher={fetcher} auth={auth} />
					</TabsContent>

					<TabsContent value="installed" className="mt-6">
						<InstalledContent />
					</TabsContent>

					<TabsContent value="projects" className="mt-6">
						<MyProjectsContent />
					</TabsContent>
				</Tabs>
			</div>
		</main>
	);
}

function PageContent() {
	const searchParams = useSearchParams();
	const packageId = searchParams.get("id");
	const auth = useAuth();
	const statusMap = usePackageStatusMap();
	const getPackageStatus = useCallback(
		(packageId: string) => statusMap.get(packageId),
		[statusMap],
	);

	if (packageId) {
		return (
			<PackageDetailWrapper
				fetcher={fetcher}
				auth={auth}
				getPackageStatus={getPackageStatus}
			/>
		);
	}

	return <PackagesHub />;
}

export default function Page() {
	return (
		<Suspense fallback={<Skeleton className="h-full w-full" />}>
			<PageContent />
		</Suspense>
	);
}
