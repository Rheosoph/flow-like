"use client";

import { invoke } from "@tauri-apps/api/core";
import { readFile } from "@tauri-apps/plugin-fs";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import {
	type PackageManifest,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Checkbox,
	Input,
	Label,
	MemoryTier,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Textarea,
	TimeoutTier,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import type { PackageInspection } from "@tm9657/flow-like-ui/lib/schema/developer";
import {
	AlertTriangle,
	ArrowLeft,
	Check,
	ChevronRight,
	Github,
	Globe,
	Loader2,
	Package,
	RefreshCw,
	Shield,
	Upload,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { post } from "../../../lib/api";

interface PublishFormData {
	id: string;
	name: string;
	version: string;
	description: string;
	license: string;
	repository: string;
	homepage: string;
	keywords: string;
	memoryTier: MemoryTier;
	timeoutTier: TimeoutTier;
	httpEnabled: boolean;
	allowedHosts: string;
	websocketEnabled: boolean;
	tcpEnabled: boolean;
	udpEnabled: boolean;
	dnsEnabled: boolean;
	nodeStorage: boolean;
	userStorage: boolean;
	variables: boolean;
	cache: boolean;
	streaming: boolean;
	a2ui: boolean;
	models: boolean;
}

type PublishStep = "manifest" | "permissions" | "review";

function camelToSnakeCase(key: string): string {
	return key.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
}

function toSnakeCaseKeys(value: unknown): unknown {
	if (Array.isArray(value)) {
		return value.map(toSnakeCaseKeys);
	}

	if (value && typeof value === "object") {
		return Object.fromEntries(
			Object.entries(value as Record<string, unknown>).map(([key, nested]) => [
				camelToSnakeCase(key),
				toSnakeCaseKeys(nested),
			]),
		);
	}

	return value;
}

function StepIndicator({
	step,
	currentStep,
	label,
}: { step: PublishStep; currentStep: PublishStep; label: string }) {
	const steps: PublishStep[] = ["manifest", "permissions", "review"];
	const currentIdx = steps.indexOf(currentStep);
	const stepIdx = steps.indexOf(step);
	const isCompleted = stepIdx < currentIdx;
	const isCurrent = step === currentStep;

	return (
		<div className="flex items-center">
			<div
				className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium ${
					isCompleted
						? "bg-primary text-primary-foreground"
						: isCurrent
							? "bg-primary text-primary-foreground"
							: "bg-muted text-muted-foreground"
				}`}
			>
				{isCompleted ? <Check className="h-4 w-4" /> : stepIdx + 1}
			</div>
			<span
				className={`ml-2 text-sm ${isCurrent ? "font-medium" : "text-muted-foreground"}`}
			>
				{label}
			</span>
		</div>
	);
}

function formFromManifest(m: PackageManifest): PublishFormData {
	const perms = m.permissions;
	const net = perms?.network;
	const fs = perms?.filesystem;
	return {
		id: m.id ?? "",
		name: m.name ?? "",
		version: m.version ?? "0.1.0",
		description: m.description ?? "",
		license: m.license ?? "MIT",
		repository: m.repository ?? "",
		homepage: m.homepage ?? "",
		keywords: (m.keywords ?? []).join(", "),
		memoryTier: perms?.memory ?? MemoryTier.Standard,
		timeoutTier: perms?.timeout ?? TimeoutTier.Standard,
		httpEnabled: net?.httpEnabled ?? false,
		allowedHosts: (net?.allowedHosts ?? []).join(", "),
		websocketEnabled: net?.websocketEnabled ?? false,
		tcpEnabled: net?.tcpEnabled ?? false,
		udpEnabled: net?.udpEnabled ?? false,
		dnsEnabled: net?.dnsEnabled ?? false,
		nodeStorage: fs?.nodeStorage ?? false,
		userStorage: fs?.userStorage ?? false,
		variables: perms?.variables ?? false,
		cache: perms?.cache ?? false,
		streaming: perms?.streaming ?? false,
		a2ui: perms?.a2ui ?? false,
		models: perms?.models ?? false,
	};
}

function DeveloperPublishPageContent() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const projectPath = searchParams.get("project");
	const auth = useAuth();
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [inspection, setInspection] = useState<PackageInspection | null>(null);
	const [manifest, setManifest] = useState<PackageManifest | null>(null);
	const [loading, setLoading] = useState(true);
	const [step, setStep] = useState<PublishStep>("manifest");
	const [publishing, setPublishing] = useState(false);
	const [idCheckState, setIdCheckState] = useState<
		"idle" | "checking" | "available" | "owned" | "taken"
	>("idle");
	const [formData, setFormData] = useState<PublishFormData>({
		id: "",
		name: "",
		version: "0.1.0",
		description: "",
		license: "MIT",
		repository: "",
		homepage: "",
		keywords: "",
		memoryTier: MemoryTier.Standard,
		timeoutTier: TimeoutTier.Standard,
		httpEnabled: false,
		allowedHosts: "",
		websocketEnabled: false,
		tcpEnabled: false,
		udpEnabled: false,
		dnsEnabled: false,
		nodeStorage: false,
		userStorage: false,
		variables: false,
		cache: false,
		streaming: false,
		a2ui: false,
		models: false,
	});

	useEffect(() => {
		if (!projectPath) return;
		let cancelled = false;
		setLoading(true);

		// Always load manifest (lightweight, no WASM required)
		const manifestLoad = invoke<PackageManifest>("developer_read_manifest", {
			projectPath,
		})
			.then((m) => {
				if (cancelled) return;
				setManifest(m);
				setFormData(formFromManifest(m));
			})
			.catch((err) => {
				if (!cancelled) toast.error(`Failed to read manifest: ${err}`);
			});

		// Also try full inspection for wasmPath (fails silently if not built)
		const inspectLoad = invoke<PackageInspection>("developer_inspect_package", {
			projectPath,
		})
			.then((result) => {
				if (cancelled) return;
				setInspection(result);
			})
			.catch(() => {
				// Not built yet — that's OK, user can still fill metadata
			});

		Promise.all([manifestLoad, inspectLoad]).finally(() => {
			if (!cancelled) setLoading(false);
		});

		return () => {
			cancelled = true;
		};
	}, [projectPath]);

	const updateField = useCallback(
		<K extends keyof PublishFormData>(key: K, value: PublishFormData[K]) => {
			setFormData((prev) => ({ ...prev, [key]: value }));
		},
		[],
	);

	const canProceed = useCallback(() => {
		switch (step) {
			case "manifest":
				return !!(
					formData.id &&
					formData.name &&
					formData.version &&
					formData.description &&
					(idCheckState === "available" || idCheckState === "owned")
				);
			case "permissions":
				return true;
			case "review":
				return true;
			default:
				return false;
		}
	}, [step, formData, idCheckState]);

	const nextStep = useCallback(() => {
		const steps: PublishStep[] = ["manifest", "permissions", "review"];
		const idx = steps.indexOf(step);
		if (idx < steps.length - 1) setStep(steps[idx + 1]);
	}, [step]);

	const prevStep = useCallback(() => {
		const steps: PublishStep[] = ["manifest", "permissions", "review"];
		const idx = steps.indexOf(step);
		if (idx > 0) setStep(steps[idx - 1]);
	}, [step]);

	const handleCheckId = useCallback(async () => {
		if (!profile.data || !formData.id) return;
		setIdCheckState("checking");
		try {
			const result = await post<{
				available: boolean;
				owned_by_caller: boolean;
			}>(profile.data.hub_profile, "registry/check-id", { id: formData.id }, auth);
			if (result.available && result.owned_by_caller) {
				setIdCheckState("owned");
			} else if (result.available) {
				setIdCheckState("available");
			} else {
				setIdCheckState("taken");
			}
		} catch {
			setIdCheckState("idle");
			toast.error("Failed to check ID availability");
		}
	}, [profile.data, formData.id, auth]);

	const handlePublish = async () => {
		if (!profile.data || !inspection) return;
		setPublishing(true);
		try {
			// Use release-mode wasm detection for publish
			const publishWasmPath = await invoke<string>("developer_find_publish_wasm", {
				projectPath,
			});

			const { upload_url } = await post<{
				upload_url: string;
				expires_in_secs: number;
			}>(profile.data.hub_profile, "registry/upload-url", {
				id: formData.id,
				version: formData.version,
			}, auth);

			const wasmBytes = await readFile(publishWasmPath);
			const uploadResponse = await tauriFetch(upload_url, {
				method: "PUT",
				headers: { "Content-Type": "application/octet-stream" },
				body: wasmBytes,
			});

			if (!uploadResponse.ok) {
				const uploadErrorBody = await uploadResponse
					.text()
					.catch(() => "<no response body>");
				throw new Error(
					`WASM upload failed (${uploadResponse.status} ${uploadResponse.statusText}): ${uploadErrorBody}`,
				);
			}

			const manifest: PackageManifest = {
				manifestVersion: 1,
				id: formData.id,
				name: formData.name,
				version: formData.version,
				description: formData.description,
				authors: auth.user?.profile.name
					? [
							{
								name: auth.user.profile.name,
								email: auth.user.profile.email,
							},
						]
					: [],
				license: formData.license || undefined,
				repository: formData.repository || undefined,
				homepage: formData.homepage || undefined,
				permissions: {
					memory: formData.memoryTier,
					timeout: formData.timeoutTier,
					network: {
						httpEnabled: formData.httpEnabled,
						allowedHosts: formData.allowedHosts
							? formData.allowedHosts.split(",").map((h) => h.trim())
							: [],
						websocketEnabled: formData.websocketEnabled,
						tcpEnabled: formData.tcpEnabled,
						udpEnabled: formData.udpEnabled,
						dnsEnabled: formData.dnsEnabled,
					},
					filesystem: {
						nodeStorage: formData.nodeStorage,
						userStorage: formData.userStorage,
						uploadDir: false,
						cacheDir: false,
					},
					oauthScopes: inspection.manifest?.permissions.oauthScopes ?? [],
					variables: formData.variables,
					cache: formData.cache,
					streaming: formData.streaming,
					a2ui: formData.a2ui,
					models: formData.models,
				},
				keywords: formData.keywords
					? formData.keywords.split(",").map((k) => k.trim())
					: [],
				minFlowLikeVersion:
					inspection.manifest?.minFlowLikeVersion ?? undefined,
				wasmPath: undefined,
				wasmHash: undefined,
				metadata: inspection.manifest?.metadata ?? {},
			};

			const response = await post<{
				success: boolean;
				package_id: string;
				version: string;
				message?: string;
			}>(profile.data.hub_profile, "registry/publish", {
				manifest: toSnakeCaseKeys(manifest),
			}, auth);

			toast.success(
				response.message ??
					"Package published as private! You can manage it from the registry.",
			);
			router.push("/store/packages?tab=projects");
		} catch (err) {
			toast.error(`Publish failed: ${err}`);
		} finally {
			setPublishing(false);
		}
	};

	if (!projectPath) {
		return (
			<div className="flex-col flex grow items-center justify-center">
				<Card className="max-w-md">
					<CardHeader>
						<CardTitle>No Project Selected</CardTitle>
						<CardDescription>
							Go back to Projects and select a project to publish.
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Button variant="outline" onClick={() => router.push("/store/packages?tab=projects")}>
							<ArrowLeft className="mr-2 h-4 w-4" />
							Back to Projects
						</Button>
					</CardContent>
				</Card>
			</div>
		);
	}

	if (!auth.isAuthenticated) {
		return (
			<div className="flex-col flex grow items-center justify-center">
				<Card className="max-w-md">
					<CardHeader>
						<CardTitle>Authentication Required</CardTitle>
						<CardDescription>
							Please sign in to publish packages to the registry.
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Button onClick={() => auth.signinRedirect()}>Sign In</Button>
					</CardContent>
				</Card>
			</div>
		);
	}

	if (loading) {
		return (
			<div className="flex-col flex grow items-center justify-center">
				<RefreshCw className="h-8 w-8 animate-spin text-muted-foreground" />
			</div>
		);
	}

	if (!manifest) {
		return (
			<div className="flex-col flex grow items-center justify-center">
				<Card className="max-w-md">
					<CardHeader>
						<CardTitle>Missing Manifest</CardTitle>
						<CardDescription>
							This project needs a <code>flow-like.toml</code> manifest before
							it can be published. Edit the manifest first.
						</CardDescription>
					</CardHeader>
					<CardContent className="flex gap-2">
						<Button
							variant="outline"
							onClick={() => router.push("/store/packages?tab=projects")}
						>
							<ArrowLeft className="mr-2 h-4 w-4" />
							Back
						</Button>
						<Button
							onClick={() =>
								router.push(
									`/developer/manifest?path=${encodeURIComponent(projectPath)}`,
								)
							}
						>
							Edit Manifest
						</Button>
					</CardContent>
				</Card>
			</div>
		);
	}

	return (
		<div className="flex-col flex grow max-h-full overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-3xl space-y-6">
				{/* Header */}
				<div className="space-y-2">
					<Button
						variant="ghost"
						size="sm"
						className="mb-2 -ml-2"
						onClick={() => router.push("/store/packages?tab=projects")}
					>
						<ArrowLeft className="mr-1 h-4 w-4" />
						Back to Projects
					</Button>
					<h1 className="text-3xl font-bold tracking-tight flex items-center gap-2">
						<Upload className="h-8 w-8" />
						Publish Package
					</h1>
					<p className="text-muted-foreground">
						Publish your local node package as a private project to the registry
					</p>
				</div>

				{/* Step Indicator */}
				<div className="flex items-center justify-between">
					<StepIndicator
						step="manifest"
						currentStep={step}
						label="Package Info"
					/>
					<ChevronRight className="h-4 w-4 text-muted-foreground" />
					<StepIndicator
						step="permissions"
						currentStep={step}
						label="Permissions"
					/>
					<ChevronRight className="h-4 w-4 text-muted-foreground" />
					<StepIndicator step="review" currentStep={step} label="Review" />
				</div>

				{/* Step Content */}
				<Card>
					{step === "manifest" && (
						<>
							<CardHeader>
								<CardTitle className="text-lg">Package Information</CardTitle>
								<CardDescription>
									Review and edit your package details before publishing
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-4">
								<div className="grid grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label htmlFor="id">Package ID *</Label>
										<div className="flex gap-2">
											<Input
												id="id"
												placeholder="com.example.my-package"
												value={formData.id}
												onChange={(e) => {
													updateField("id", e.target.value);
													setIdCheckState("idle");
												}}
												className="flex-1"
											/>
											<Button
												type="button"
												variant={
													idCheckState === "available" || idCheckState === "owned"
														? "outline"
														: "secondary"
												}
												size="sm"
												disabled={!formData.id || idCheckState === "checking"}
												onClick={handleCheckId}
												className="shrink-0"
											>
												{idCheckState === "checking" ? (
													<Loader2 className="h-4 w-4 animate-spin" />
												) : idCheckState === "available" || idCheckState === "owned" ? (
													<Check className="h-4 w-4 text-green-500" />
												) : idCheckState === "taken" ? (
													<AlertTriangle className="h-4 w-4 text-destructive" />
												) : (
													"Check"
												)}
											</Button>
										</div>
										{idCheckState === "available" && (
											<p className="text-xs text-green-500">ID is available</p>
										)}
										{idCheckState === "owned" && (
											<p className="text-xs text-green-500">You own this package</p>
										)}
										{idCheckState === "taken" && (
											<p className="text-xs text-destructive">ID is already taken by another user</p>
										)}
									</div>
									<div className="space-y-2">
										<Label htmlFor="version">Version *</Label>
										<Input
											id="version"
											placeholder="1.0.0"
											value={formData.version}
											onChange={(e) => updateField("version", e.target.value)}
										/>
									</div>
								</div>
								<div className="space-y-2">
									<Label htmlFor="name">Display Name *</Label>
									<Input
										id="name"
										placeholder="My Awesome Package"
										value={formData.name}
										onChange={(e) => updateField("name", e.target.value)}
									/>
								</div>
								<div className="space-y-2">
									<Label htmlFor="description">Description *</Label>
									<Textarea
										id="description"
										placeholder="A brief description of what your package does..."
										value={formData.description}
										onChange={(e) => updateField("description", e.target.value)}
										rows={3}
									/>
								</div>
								<Separator />
								<div className="grid grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label htmlFor="license">License</Label>
										<Input
											id="license"
											placeholder="MIT"
											value={formData.license}
											onChange={(e) => updateField("license", e.target.value)}
										/>
									</div>
									<div className="space-y-2">
										<Label htmlFor="keywords">Keywords</Label>
										<Input
											id="keywords"
											placeholder="ai, data, transform"
											value={formData.keywords}
											onChange={(e) => updateField("keywords", e.target.value)}
										/>
									</div>
								</div>
								<div className="grid grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label
											htmlFor="repository"
											className="flex items-center gap-1"
										>
											<Github className="h-4 w-4" />
											Repository URL
										</Label>
										<Input
											id="repository"
											placeholder="https://github.com/..."
											value={formData.repository}
											onChange={(e) =>
												updateField("repository", e.target.value)
											}
										/>
									</div>
									<div className="space-y-2">
										<Label
											htmlFor="homepage"
											className="flex items-center gap-1"
										>
											<Globe className="h-4 w-4" />
											Homepage
										</Label>
										<Input
											id="homepage"
											placeholder="https://..."
											value={formData.homepage}
											onChange={(e) => updateField("homepage", e.target.value)}
										/>
									</div>
								</div>
							</CardContent>
						</>
					)}

					{step === "permissions" && (
						<>
							<CardHeader>
								<CardTitle className="text-lg">
									Permissions & Resources
								</CardTitle>
								<CardDescription>
									Declare the capabilities your package needs
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-6">
								<div className="grid grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label>Memory Tier</Label>
										<Select
											value={formData.memoryTier}
											onValueChange={(v) =>
												updateField("memoryTier", v as MemoryTier)
											}
										>
											<SelectTrigger>
												<SelectValue />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="minimal">
													Minimal (16 MB)
												</SelectItem>
												<SelectItem value="light">Light (32 MB)</SelectItem>
												<SelectItem value="standard">
													Standard (64 MB)
												</SelectItem>
												<SelectItem value="heavy">Heavy (128 MB)</SelectItem>
												<SelectItem value="intensive">
													Intensive (256 MB)
												</SelectItem>
												<SelectItem value="large">Large (512 MB)</SelectItem>
												<SelectItem value="huge">Huge (1 GB)</SelectItem>
												<SelectItem value="extreme">Extreme (2 GB)</SelectItem>
												<SelectItem value="maximum">Maximum (4 GB)</SelectItem>
											</SelectContent>
										</Select>
									</div>
									<div className="space-y-2">
										<Label>Timeout Tier</Label>
										<Select
											value={formData.timeoutTier}
											onValueChange={(v) =>
												updateField("timeoutTier", v as TimeoutTier)
											}
										>
											<SelectTrigger>
												<SelectValue />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="quick">Quick (5s)</SelectItem>
												<SelectItem value="standard">
													Standard (30s)
												</SelectItem>
												<SelectItem value="extended">
													Extended (60s)
												</SelectItem>
												<SelectItem value="long_running">
													Long Running (5min)
												</SelectItem>
												<SelectItem value="very_long">
													Very Long (10min)
												</SelectItem>
												<SelectItem value="maximum">
													Maximum (30min)
												</SelectItem>
											</SelectContent>
										</Select>
									</div>
								</div>

								<Separator />

								<div className="space-y-4">
									<h4 className="font-medium">Network Access</h4>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="httpEnabled"
											checked={formData.httpEnabled}
											onCheckedChange={(c) =>
												updateField("httpEnabled", c === true)
											}
										/>
										<Label htmlFor="httpEnabled">Enable HTTP requests</Label>
									</div>
									{formData.httpEnabled && (
										<div className="space-y-2 ml-6">
											<Label htmlFor="allowedHosts">
												Allowed Hosts (comma-separated, empty = all)
											</Label>
											<Input
												id="allowedHosts"
												placeholder="api.example.com, cdn.example.com"
												value={formData.allowedHosts}
												onChange={(e) =>
													updateField("allowedHosts", e.target.value)
												}
											/>
										</div>
									)}
									<div className="flex items-center space-x-2">
										<Checkbox
											id="websocketEnabled"
											checked={formData.websocketEnabled}
											onCheckedChange={(c) =>
												updateField("websocketEnabled", c === true)
											}
										/>
										<Label htmlFor="websocketEnabled">
											Enable WebSocket connections
										</Label>
									</div>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="tcpEnabled"
											checked={formData.tcpEnabled}
											onCheckedChange={(c) =>
												updateField("tcpEnabled", c === true)
											}
										/>
										<Label htmlFor="tcpEnabled">
											Enable TCP sockets
										</Label>
									</div>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="udpEnabled"
											checked={formData.udpEnabled}
											onCheckedChange={(c) =>
												updateField("udpEnabled", c === true)
											}
										/>
										<Label htmlFor="udpEnabled">
											Enable UDP sockets
										</Label>
									</div>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="dnsEnabled"
											checked={formData.dnsEnabled}
											onCheckedChange={(c) =>
												updateField("dnsEnabled", c === true)
											}
										/>
										<Label htmlFor="dnsEnabled">
											Enable DNS lookups
										</Label>
									</div>
								</div>

								<Separator />

								<div className="space-y-4">
									<h4 className="font-medium">Storage Access</h4>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="nodeStorage"
											checked={formData.nodeStorage}
											onCheckedChange={(c) =>
												updateField("nodeStorage", c === true)
											}
										/>
										<Label htmlFor="nodeStorage">Node-scoped storage</Label>
									</div>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="userStorage"
											checked={formData.userStorage}
											onCheckedChange={(c) =>
												updateField("userStorage", c === true)
											}
										/>
										<Label htmlFor="userStorage">User-scoped storage</Label>
									</div>
								</div>

								<Separator />

								<div className="space-y-4">
									<h4 className="font-medium">Additional Capabilities</h4>
									<div className="grid grid-cols-2 gap-2">
										<div className="flex items-center space-x-2">
											<Checkbox
												id="variables"
												checked={formData.variables}
												onCheckedChange={(c) =>
													updateField("variables", c === true)
												}
											/>
											<Label htmlFor="variables">Variables</Label>
										</div>
										<div className="flex items-center space-x-2">
											<Checkbox
												id="cache"
												checked={formData.cache}
												onCheckedChange={(c) =>
													updateField("cache", c === true)
												}
											/>
											<Label htmlFor="cache">Cache</Label>
										</div>
										<div className="flex items-center space-x-2">
											<Checkbox
												id="streaming"
												checked={formData.streaming}
												onCheckedChange={(c) =>
													updateField("streaming", c === true)
												}
											/>
											<Label htmlFor="streaming">Streaming</Label>
										</div>
										<div className="flex items-center space-x-2">
											<Checkbox
												id="a2ui"
												checked={formData.a2ui}
												onCheckedChange={(c) =>
													updateField("a2ui", c === true)
												}
											/>
											<Label htmlFor="a2ui">A2UI</Label>
										</div>
										<div className="flex items-center space-x-2">
											<Checkbox
												id="models"
												checked={formData.models}
												onCheckedChange={(c) =>
													updateField("models", c === true)
												}
											/>
											<Label htmlFor="models">Models / LLM</Label>
										</div>
									</div>
								</div>
							</CardContent>
						</>
					)}

					{step === "review" && (
						<>
							<CardHeader>
								<CardTitle className="text-lg">Review & Submit</CardTitle>
								<CardDescription>
									Review your package details before submitting
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-6">
								<div className="rounded-lg border p-4 space-y-3">
									<div className="flex items-center gap-2">
										<Package className="h-5 w-5" />
										<span className="font-semibold">{formData.name}</span>
										<Badge variant="outline">v{formData.version}</Badge>
									</div>
									<p className="text-sm text-muted-foreground">
										{formData.description}
									</p>
									<p className="text-xs text-muted-foreground">
										ID: {formData.id}
									</p>
									{formData.keywords && (
										<div className="flex flex-wrap gap-1">
											{formData.keywords
												.split(",")
												.filter(Boolean)
												.map((kw) => (
													<Badge
														key={kw}
														variant="secondary"
														className="text-xs"
													>
														{kw.trim()}
													</Badge>
												))}
										</div>
									)}
								</div>

								{inspection && inspection.nodes.length > 0 && (
									<div className="rounded-lg border p-4 space-y-2">
										<h4 className="font-medium text-sm">
											{inspection.nodes.length} Node(s)
										</h4>
										<div className="flex flex-wrap gap-1">
											{inspection.nodes.map((node) => (
												<Badge
													key={node.name}
													variant="secondary"
													className="text-xs gap-1"
												>
													{node.icon && <span>{node.icon}</span>}
													{node.friendly_name}
												</Badge>
											))}
										</div>
									</div>
								)}

								<div className="rounded-lg border p-4 space-y-2">
									<h4 className="font-medium flex items-center gap-2">
										<Shield className="h-4 w-4" />
										Permissions Summary
									</h4>
									<div className="flex flex-wrap gap-1">
										<Badge variant="outline">
											Memory: {formData.memoryTier}
										</Badge>
										<Badge variant="outline">
											Timeout: {formData.timeoutTier}
										</Badge>
										{formData.httpEnabled && (
											<Badge variant="outline">HTTP</Badge>
										)}
										{formData.websocketEnabled && (
											<Badge variant="outline">WebSocket</Badge>
										)}
										{formData.tcpEnabled && (
											<Badge variant="outline">TCP</Badge>
										)}
										{formData.udpEnabled && (
											<Badge variant="outline">UDP</Badge>
										)}
										{formData.dnsEnabled && (
											<Badge variant="outline">DNS</Badge>
										)}
										{formData.nodeStorage && (
											<Badge variant="outline">Node Storage</Badge>
										)}
										{formData.userStorage && (
											<Badge variant="outline">User Storage</Badge>
										)}
										{formData.variables && (
											<Badge variant="outline">Variables</Badge>
										)}
										{formData.cache && <Badge variant="outline">Cache</Badge>}
										{formData.streaming && (
											<Badge variant="outline">Streaming</Badge>
										)}
										{formData.a2ui && <Badge variant="outline">A2UI</Badge>}
										{formData.models && (
											<Badge variant="outline">Models</Badge>
										)}
									</div>
								</div>

							{!inspection?.wasmPath && (
								<div className="rounded-lg bg-destructive/10 border border-destructive/20 p-4">
									<div className="flex items-start gap-2">
										<AlertTriangle className="h-5 w-5 text-destructive mt-0.5" />
										<div>
											<h4 className="font-medium text-destructive">Build Required</h4>
											<p className="text-sm text-muted-foreground mt-1">
												No compiled WASM found. Run <code>mise run build</code> in
												your project directory before publishing.
											</p>
										</div>
									</div>
								</div>
							)}

							<div className="rounded-lg bg-yellow-500/10 border border-yellow-500/20 p-4">
								<div className="flex items-start gap-2">
									<AlertTriangle className="h-5 w-5 text-yellow-500 mt-0.5" />
									<div>
										<h4 className="font-medium text-yellow-500">
											Private Package
										</h4>
										<p className="text-sm text-muted-foreground mt-1">
											Your package will be published as a private project. You
											can request public visibility later from the registry
											management page.
										</p>
									</div>
								</div>
							</div>
							</CardContent>
						</>
					)}

					{/* Navigation */}
					<div className="flex justify-between p-6 pt-0">
						<Button
							variant="outline"
							onClick={step === "manifest" ? () => router.push("/store/packages?tab=projects") : prevStep}
						>
							Back
						</Button>
						{step === "review" ? (
							<Button onClick={handlePublish} disabled={publishing || !inspection?.wasmPath}>
								{publishing ? (
									<>
										<RefreshCw className="mr-2 h-4 w-4 animate-spin" />
										Publishing...
									</>
								) : (
									<>
										<Upload className="mr-2 h-4 w-4" />
										Publish as Private
									</>
								)}
							</Button>
						) : (
							<Button onClick={nextStep} disabled={!canProceed()}>
								Continue
								<ChevronRight className="ml-2 h-4 w-4" />
							</Button>
						)}
					</div>
				</Card>
			</div>
		</div>
	);
}

export default function DeveloperPublishPage() {
	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full">
					<RefreshCw className="h-6 w-6 animate-spin text-muted-foreground/60" />
				</div>
			}
		>
			<DeveloperPublishPageContent />
		</Suspense>
	);
}
