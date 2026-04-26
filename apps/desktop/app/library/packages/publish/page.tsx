"use client";

import { open } from "@tauri-apps/plugin-dialog";
import { readFile } from "@tauri-apps/plugin-fs";
import {
	MemoryTier,
	type PackageManifest,
	TimeoutTier,
	useBackend,
	useInvoke,
	useMutation,
} from "@tm9657/flow-like-ui";
import {
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
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Textarea,
} from "@tm9657/flow-like-ui/components";
import {
	AlertTriangle,
	Check,
	ChevronRight,
	FileCode,
	Github,
	Globe,
	Package,
	RefreshCw,
	Shield,
	Upload,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { post } from "../../../../lib/api";
import { toSnakeCaseKeys } from "../../../../lib/snake-case";

interface PublishFormData {
	id: string;
	name: string;
	version: string;
	description: string;
	license: string;
	repository: string;
	homepage: string;
	keywords: string;
	// Permissions
	memoryTier: MemoryTier;
	timeoutTier: TimeoutTier;
	httpEnabled: boolean;
	allowedHosts: string;
	websocketEnabled: boolean;
	nodeStorage: boolean;
	userStorage: boolean;
	variables: boolean;
	cache: boolean;
	streaming: boolean;
	a2ui: boolean;
	models: boolean;
}

type PublishStep = "upload" | "manifest" | "permissions" | "review";

function StepIndicator({
	step,
	currentStep,
	label,
}: { step: PublishStep; currentStep: PublishStep; label: string }) {
	const steps: PublishStep[] = ["upload", "manifest", "permissions", "review"];
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

export default function PublishPackagePage() {
	const router = useRouter();
	const auth = useAuth();
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [step, setStep] = useState<PublishStep>("upload");
	const [wasmFile, setWasmFile] = useState<{
		path: string;
		data: Uint8Array;
	} | null>(null);
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
		nodeStorage: false,
		userStorage: false,
		variables: false,
		cache: false,
		streaming: false,
		a2ui: false,
		models: false,
	});

	const selectWasmFile = useCallback(async () => {
		const selected = await open({
			filters: [{ name: "WASM", extensions: ["wasm"] }],
			multiple: false,
		});

		if (selected) {
			const data = await readFile(selected);
			// Validate WASM magic bytes
			if (
				data.length < 8 ||
				data[0] !== 0 ||
				data[1] !== 0x61 ||
				data[2] !== 0x73 ||
				data[3] !== 0x6d
			) {
				toast.error("Invalid WASM file");
				return;
			}
			setWasmFile({ path: selected, data });
			// Extract filename for package id
			const filename = selected.split("/").pop()?.replace(".wasm", "") ?? "";
			setFormData((prev) => ({
				...prev,
				id: filename.replace(/_/g, "-"),
				name: filename.replace(/_/g, " ").replace(/-/g, " "),
			}));
			setStep("manifest");
			toast.success("WASM file loaded successfully");
		}
	}, []);

	const updateField = useCallback(
		<K extends keyof PublishFormData>(key: K, value: PublishFormData[K]) => {
			setFormData((prev) => ({ ...prev, [key]: value }));
		},
		[],
	);

	const publishMutation = useMutation({
		mutationFn: async () => {
			if (!profile.data || !wasmFile) {
				throw new Error("Missing profile or WASM file");
			}

			// Build manifest
			const manifest: PackageManifest = {
				manifestVersion: 1,
				id: formData.id,
				name: formData.name,
				version: formData.version,
				description: formData.description,
				authors: auth.user?.profile.name
					? [{ name: auth.user.profile.name, email: auth.user.profile.email }]
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
						tcpEnabled: false,
						udpEnabled: false,
						dnsEnabled: false,
					},
					filesystem: {
						nodeStorage: formData.nodeStorage,
						userStorage: formData.userStorage,
						uploadDir: false,
						cacheDir: false,
					},
					oauthScopes: [],
					variables: formData.variables,
					cache: formData.cache,
					streaming: formData.streaming,
					a2ui: formData.a2ui,
					models: formData.models,
				},
				keywords: formData.keywords
					? formData.keywords.split(",").map((k) => k.trim())
					: [],
				minFlowLikeVersion: undefined,
				wasmPath: undefined,
				wasmHash: undefined,
				metadata: {},
			};

			// Convert Uint8Array to base64
			const base64 = btoa(
				Array.from(wasmFile.data)
					.map((b) => String.fromCharCode(b))
					.join(""),
			);

			return post<{
				success: boolean;
				package_id: string;
				version: string;
				message?: string;
			}>(
				profile.data.hub_profile,
				"registry/publish",
				{
					manifest: toSnakeCaseKeys(manifest),
					wasm_base64: base64,
				},
				auth,
			);
		},
		onSuccess: (response: {
			success: boolean;
			package_id: string;
			version: string;
			message?: string;
		}) => {
			toast.success(
				response.message ??
					"Package submitted for review! It will be available after admin approval.",
			);
			router.push("/library/packages");
		},
		onError: (error: Error) => {
			toast.error(`Failed to publish: ${error.message}`);
		},
	});

	const canProceed = useCallback(() => {
		switch (step) {
			case "upload":
				return !!wasmFile;
			case "manifest":
				return !!(
					formData.id &&
					formData.name &&
					formData.version &&
					formData.description
				);
			case "permissions":
				return true;
			case "review":
				return true;
			default:
				return false;
		}
	}, [step, wasmFile, formData]);

	const nextStep = useCallback(() => {
		const steps: PublishStep[] = [
			"upload",
			"manifest",
			"permissions",
			"review",
		];
		const idx = steps.indexOf(step);
		if (idx < steps.length - 1) {
			setStep(steps[idx + 1]);
		}
	}, [step]);

	const prevStep = useCallback(() => {
		const steps: PublishStep[] = [
			"upload",
			"manifest",
			"permissions",
			"review",
		];
		const idx = steps.indexOf(step);
		if (idx > 0) {
			setStep(steps[idx - 1]);
		}
	}, [step]);

	if (!auth.isAuthenticated) {
		return (
			<main className="flex-col flex flex-grow max-h-full p-6 overflow-auto items-center justify-center">
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
			</main>
		);
	}

	return (
		<main className="flex-col flex flex-grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-3xl space-y-6">
				{/* Header */}
				<div className="space-y-2">
					<h1 className="text-3xl font-bold tracking-tight flex items-center gap-2">
						<Upload className="h-8 w-8" />
						Publish Package
					</h1>
					<p className="text-muted-foreground">
						Share your WASM node package with the community
					</p>
				</div>

				{/* Step Indicator */}
				<div className="flex items-center justify-between">
					<StepIndicator step="upload" currentStep={step} label="Upload WASM" />
					<ChevronRight className="h-4 w-4 text-muted-foreground" />
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
					{step === "upload" && (
						<>
							<CardHeader>
								<CardTitle className="text-lg">Upload WASM File</CardTitle>
								<CardDescription>
									Select the compiled WASM file for your node package
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-4">
								<div
									className="border-2 border-dashed rounded-lg p-12 text-center cursor-pointer hover:border-primary transition-colors"
									onClick={selectWasmFile}
								>
									{wasmFile ? (
										<div className="space-y-2">
											<FileCode className="mx-auto h-12 w-12 text-primary" />
											<p className="font-medium">
												{wasmFile.path.split("/").pop()}
											</p>
											<p className="text-sm text-muted-foreground">
												{(wasmFile.data.length / 1024).toFixed(2)} KB
											</p>
											<Button variant="outline" size="sm">
												Choose Different File
											</Button>
										</div>
									) : (
										<div className="space-y-2">
											<Upload className="mx-auto h-12 w-12 text-muted-foreground" />
											<p className="font-medium">Click to select WASM file</p>
											<p className="text-sm text-muted-foreground">
												Only .wasm files are accepted
											</p>
										</div>
									)}
								</div>
							</CardContent>
						</>
					)}

					{step === "manifest" && (
						<>
							<CardHeader>
								<CardTitle className="text-lg">Package Information</CardTitle>
								<CardDescription>
									Provide details about your package
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-4">
								<div className="grid grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label htmlFor="id">Package ID *</Label>
										<Input
											id="id"
											placeholder="com.example.my-package"
											value={formData.id}
											onChange={(e) => updateField("id", e.target.value)}
										/>
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
												updateField(
													"memoryTier",
													v as PublishFormData["memoryTier"],
												)
											}
										>
											<SelectTrigger>
												<SelectValue />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="minimal">Minimal (16 MB)</SelectItem>
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
												updateField(
													"timeoutTier",
													v as PublishFormData["timeoutTier"],
												)
											}
										>
											<SelectTrigger>
												<SelectValue />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="quick">Quick (5s)</SelectItem>
												<SelectItem value="standard">Standard (30s)</SelectItem>
												<SelectItem value="extended">Extended (60s)</SelectItem>
												<SelectItem value="long_running">
													Long Running (5min)
												</SelectItem>
												<SelectItem value="very_long">
													Very Long (10min)
												</SelectItem>
												<SelectItem value="maximum">Maximum (30min)</SelectItem>
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
												onCheckedChange={(c) => updateField("a2ui", c === true)}
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
									Review your package details before submitting for review
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
									<div className="flex flex-wrap gap-1">
										{formData.keywords
											.split(",")
											.filter(Boolean)
											.map((kw) => (
												<Badge key={kw} variant="secondary" className="text-xs">
													{kw.trim()}
												</Badge>
											))}
									</div>
								</div>

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
										{formData.models && <Badge variant="outline">Models</Badge>}
									</div>
								</div>

								<div className="rounded-lg bg-yellow-500/10 border border-yellow-500/20 p-4">
									<div className="flex items-start gap-2">
										<AlertTriangle className="h-5 w-5 text-yellow-500 mt-0.5" />
										<div>
											<h4 className="font-medium text-yellow-500">
												Admin Review Required
											</h4>
											<p className="text-sm text-muted-foreground mt-1">
												Your package will be submitted for review. An admin will
												review the code and permissions before it becomes
												available in the public registry.
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
							onClick={prevStep}
							disabled={step === "upload"}
						>
							Back
						</Button>
						{step === "review" ? (
							<Button
								onClick={() => publishMutation.mutate()}
								disabled={publishMutation.isPending}
							>
								{publishMutation.isPending ? (
									<>
										<RefreshCw className="mr-2 h-4 w-4 animate-spin" />
										Publishing...
									</>
								) : (
									<>
										<Upload className="mr-2 h-4 w-4" />
										Submit for Review
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
		</main>
	);
}
