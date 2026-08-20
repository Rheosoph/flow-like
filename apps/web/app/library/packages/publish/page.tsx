"use client";

import {
	MemoryTier,
	type PackageManifest,
	TimeoutTier,
	useBackend,
	useInvoke,
	useMutation,
} from "@flow-like/flow-like-ui";
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
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	Check,
	ChevronRight,
	FileCode,
	Github,
	Globe,
	Loader2,
	Package,
	RefreshCw,
	Shield,
	Upload,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { post } from "../../../../lib/api";
import { currentRelativeUrl } from "../../../../lib/return-url";

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

function bumpPatch(version: string): string {
	const parts = version.split(".");
	if (parts.length !== 3) return version;
	const patch = Number.parseInt(parts[2], 10);
	if (Number.isNaN(patch)) return version;
	return `${parts[0]}.${parts[1]}.${patch + 1}`;
}

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
	const { t } = useTranslation("common");
	const router = useRouter();
	const auth = useAuth();
	const backend = useBackend();
	const fileInputRef = useRef<HTMLInputElement>(null);
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [step, setStep] = useState<PublishStep>("upload");
	const [versionCheckState, setVersionCheckState] = useState<
		"idle" | "checking" | "available" | "taken" | "bumped"
	>("idle");
	const [wasmFile, setWasmFile] = useState<{
		name: string;
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

	const handleFileSelect = useCallback(
		async (event: React.ChangeEvent<HTMLInputElement>) => {
			const file = event.target.files?.[0];
			if (!file) return;

			const buffer = await file.arrayBuffer();
			const data = new Uint8Array(buffer);

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

			setWasmFile({ name: file.name, data });
			// Extract filename for package id
			const filename = file.name.replace(".wasm", "") ?? "";
			setFormData((prev) => ({
				...prev,
				id: filename.replace(/_/g, "-"),
				name: filename.replace(/_/g, " ").replace(/-/g, " "),
			}));
			setStep("manifest");
			toast.success("WASM file loaded successfully");
		},
		[],
	);

	const selectWasmFile = useCallback(async () => {
		fileInputRef.current?.click();
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

			const result = await post<{
				success: boolean;
				package_id: string;
				version: string;
				message?: string;
			}>(
				profile.data.hub_profile,
				"registry/publish",
				{
					manifest,
					wasm_base64: base64,
				},
				auth,
			);

			if (!result) {
				throw new Error("Failed to publish package");
			}

			return result;
		},
		onSuccess: (response: {
			success: boolean;
			package_id: string;
			version: string;
			message?: string;
		}) => {
			toast.success(
				response.message ??
					t(
						"packageSubmittedForReviewItWillBeAvailableAfterAdminApproval",
						"Package submitted for review! It will be available after admin approval.",
					),
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
					formData.description &&
					(versionCheckState === "available" || versionCheckState === "bumped")
				);
			case "permissions":
				return true;
			case "review":
				return true;
			default:
				return false;
		}
	}, [step, wasmFile, formData, versionCheckState]);

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

	const handleCheckVersion = useCallback(async () => {
		if (!profile.data || !formData.id || !formData.version) return;
		setVersionCheckState("checking");
		try {
			const result = await post<{ available: boolean }>(
				profile.data.hub_profile,
				"registry/check-version",
				{ id: formData.id, version: formData.version },
				auth,
			);
			if (result?.available) {
				setVersionCheckState("available");
			} else {
				const bumped = bumpPatch(formData.version);
				setFormData((prev) => ({ ...prev, version: bumped }));
				setVersionCheckState("bumped");
				toast.info(
					t(
						"versionVersionAlreadyExistsBumpedToBumped",
						"Version {{version}} already exists — bumped to {{bumped}}",
						{ version: formData.version, bumped },
					),
				);
			}
		} catch {
			setVersionCheckState("idle");
			toast.error("Failed to check version availability");
		}
	}, [profile.data, formData.id, formData.version, auth]);

	if (!auth.isAuthenticated) {
		return (
			<main className="flex-col flex flex-grow max-h-full p-6 overflow-auto items-center justify-center">
				<Card className="max-w-md">
					<CardHeader>
						<CardTitle>
							{t("authenticationRequired", "Authentication Required")}
						</CardTitle>
						<CardDescription>
							{t(
								"pleaseSignInToPublishPackagesToTheRegistry",
								"Please sign in to publish packages to the registry.",
							)}
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Button
							onClick={() =>
								auth.signinRedirect({ url_state: currentRelativeUrl() })
							}
						>
							{t("signIn", "Sign In")}
						</Button>
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
						{t("publishPackage", "Publish Package")}
					</h1>
					<p className="text-muted-foreground">
						{t(
							"shareYourWasmNodePackageWithTheCommunity",
							"Share your WASM node package with the community",
						)}
					</p>
				</div>

				{/* Step Indicator */}
				<div className="flex items-center justify-between">
					<StepIndicator
						step="upload"
						currentStep={step}
						label={t("uploadWasm", "Upload WASM")}
					/>
					<ChevronRight className="h-4 w-4 text-muted-foreground" />
					<StepIndicator
						step="manifest"
						currentStep={step}
						label={t("packageInfo", "Package Info")}
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
								<CardTitle className="text-lg">
									{t("uploadWasmFile", "Upload WASM File")}
								</CardTitle>
								<CardDescription>
									{t(
										"selectTheCompiledWasmFileForYourNodePackage",
										"Select the compiled WASM file for your node package",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-4">
								<input
									type="file"
									ref={fileInputRef}
									accept=".wasm"
									onChange={handleFileSelect}
									className="hidden"
								/>
								<div
									className="border-2 border-dashed rounded-lg p-12 text-center cursor-pointer hover:border-primary transition-colors"
									onClick={selectWasmFile}
								>
									{wasmFile ? (
										<div className="space-y-2">
											<FileCode className="mx-auto h-12 w-12 text-primary" />
											<p className="font-medium">{wasmFile.name}</p>
											<p className="text-sm text-muted-foreground">
												{(wasmFile.data.length / 1024).toFixed(2)} KB
											</p>
											<Button variant="outline" size="sm">
												{t("chooseDifferentFile", "Choose Different File")}
											</Button>
										</div>
									) : (
										<div className="space-y-2">
											<Upload className="mx-auto h-12 w-12 text-muted-foreground" />
											<p className="font-medium">
												{t(
													"clickToSelectWasmFile",
													"Click to select WASM file",
												)}
											</p>
											<p className="text-sm text-muted-foreground">
												{t(
													"onlyWasmFilesAreAccepted",
													"Only .wasm files are accepted",
												)}
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
								<CardTitle className="text-lg">
									{t("packageInformation", "Package Information")}
								</CardTitle>
								<CardDescription>
									{t(
										"provideDetailsAboutYourPackage",
										"Provide details about your package",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-4">
								<div className="grid grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label htmlFor="id">{t("packageId", "Package ID *")}</Label>
										<Input
											id="id"
											placeholder="com.example.my-package"
											value={formData.id}
											onChange={(e) => updateField("id", e.target.value)}
										/>
									</div>
									<div className="space-y-2">
										<Label htmlFor="version">
											{t("version2", "Version *")}
										</Label>
										<div className="flex gap-2">
											<Input
												id="version"
												placeholder="1.0.0"
												value={formData.version}
												onChange={(e) => {
													updateField("version", e.target.value);
													setVersionCheckState("idle");
												}}
												className="flex-1"
											/>
											<Button
												type="button"
												variant={
													versionCheckState === "available" ||
													versionCheckState === "bumped"
														? "outline"
														: "secondary"
												}
												size="sm"
												disabled={
													!formData.version ||
													!formData.id ||
													versionCheckState === "checking"
												}
												onClick={handleCheckVersion}
												className="shrink-0"
											>
												{versionCheckState === "checking" ? (
													<Loader2 className="h-4 w-4 animate-spin" />
												) : versionCheckState === "available" ||
													versionCheckState === "bumped" ? (
													<Check className="h-4 w-4 text-green-500" />
												) : (
													"Check"
												)}
											</Button>
										</div>
										{versionCheckState === "available" && (
											<p className="text-xs text-green-500">
												{t("versionIsAvailable", "Version is available")}
											</p>
										)}
										{versionCheckState === "bumped" && (
											<p className="text-xs text-yellow-500">
												{t(
													"autobumpedToAvailableVersion",
													"Auto-bumped to available version",
												)}
											</p>
										)}
									</div>
								</div>
								<div className="space-y-2">
									<Label htmlFor="name">
										{t("displayName3", "Display Name *")}
									</Label>
									<Input
										id="name"
										placeholder={t("myAwesomePackage", "My Awesome Package")}
										value={formData.name}
										onChange={(e) => updateField("name", e.target.value)}
									/>
								</div>
								<div className="space-y-2">
									<Label htmlFor="description">
										{t("description2", "Description *")}
									</Label>
									<Textarea
										id="description"
										placeholder={t(
											"aBriefDescriptionOfWhatYourPackageDoes",
											"A brief description of what your package does...",
										)}
										value={formData.description}
										onChange={(e) => updateField("description", e.target.value)}
										rows={3}
									/>
								</div>
								<Separator />
								<div className="grid grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label htmlFor="license">{t("license", "License")}</Label>
										<Input
											id="license"
											placeholder="MIT"
											value={formData.license}
											onChange={(e) => updateField("license", e.target.value)}
										/>
									</div>
									<div className="space-y-2">
										<Label htmlFor="keywords">
											{t("keywords", "Keywords")}
										</Label>
										<Input
											id="keywords"
											placeholder={t("aiDataTransform", "ai, data, transform")}
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
											{t("repositoryUrl", "Repository URL")}
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
											{t("homepage", "Homepage")}
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
									{t("permissionsResources", "Permissions & Resources")}
								</CardTitle>
								<CardDescription>
									{t(
										"declareTheCapabilitiesYourPackageNeeds",
										"Declare the capabilities your package needs",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-6">
								<div className="grid grid-cols-2 gap-4">
									<div className="space-y-2">
										<Label>{t("memoryTier", "Memory Tier")}</Label>
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
												<SelectItem value="minimal">
													{t("minimal16Mb", "Minimal (16 MB)")}
												</SelectItem>
												<SelectItem value="light">
													{t("light32Mb", "Light (32 MB)")}
												</SelectItem>
												<SelectItem value="standard">
													{t("standard64Mb", "Standard (64 MB)")}
												</SelectItem>
												<SelectItem value="heavy">
													{t("heavy128Mb", "Heavy (128 MB)")}
												</SelectItem>
												<SelectItem value="intensive">
													{t("intensive256Mb", "Intensive (256 MB)")}
												</SelectItem>
												<SelectItem value="large">
													{t("large512Mb", "Large (512 MB)")}
												</SelectItem>
												<SelectItem value="huge">
													{t("huge1Gb", "Huge (1 GB)")}
												</SelectItem>
												<SelectItem value="extreme">
													{t("extreme2Gb", "Extreme (2 GB)")}
												</SelectItem>
												<SelectItem value="maximum">
													{t("maximum4Gb", "Maximum (4 GB)")}
												</SelectItem>
											</SelectContent>
										</Select>
									</div>
									<div className="space-y-2">
										<Label>{t("timeoutTier", "Timeout Tier")}</Label>
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
												<SelectItem value="quick">
													{t("quick5s", "Quick (5s)")}
												</SelectItem>
												<SelectItem value="standard">
													{t("standard30s", "Standard (30s)")}
												</SelectItem>
												<SelectItem value="extended">
													{t("extended60s", "Extended (60s)")}
												</SelectItem>
												<SelectItem value="long_running">
													{t("longRunning5min", "Long Running (5min)")}
												</SelectItem>
												<SelectItem value="very_long">
													{t("veryLong10min", "Very Long (10min)")}
												</SelectItem>
												<SelectItem value="maximum">
													{t("maximum30min", "Maximum (30min)")}
												</SelectItem>
											</SelectContent>
										</Select>
									</div>
								</div>

								<Separator />

								<div className="space-y-4">
									<h4 className="font-medium">
										{t("networkAccess", "Network Access")}
									</h4>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="httpEnabled"
											checked={formData.httpEnabled}
											onCheckedChange={(c) =>
												updateField("httpEnabled", c === true)
											}
										/>
										<Label htmlFor="httpEnabled">
											{t("enableHttpRequests", "Enable HTTP requests")}
										</Label>
									</div>
									{formData.httpEnabled && (
										<div className="space-y-2 ml-6">
											<Label htmlFor="allowedHosts">
												{t(
													"allowedHostsCommaseparatedEmptyAll",
													"Allowed Hosts (comma-separated, empty = all)",
												)}
											</Label>
											<Input
												id="allowedHosts"
												placeholder={t(
													"apiexamplecomCdnexamplecom",
													"api.example.com, cdn.example.com",
												)}
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
												updateField(`websocketEnabled`, c === true)
											}
										/>
										<Label htmlFor="websocketEnabled">
											{t(
												"enableWebsocketConnections",
												"Enable WebSocket connections",
											)}
										</Label>
									</div>
								</div>

								<Separator />

								<div className="space-y-4">
									<h4 className="font-medium">
										{t("storageAccess", "Storage Access")}
									</h4>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="nodeStorage"
											checked={formData.nodeStorage}
											onCheckedChange={(c) =>
												updateField("nodeStorage", c === true)
											}
										/>
										<Label htmlFor="nodeStorage">
											{t("nodescopedStorage", "Node-scoped storage")}
										</Label>
									</div>
									<div className="flex items-center space-x-2">
										<Checkbox
											id="userStorage"
											checked={formData.userStorage}
											onCheckedChange={(c) =>
												updateField("userStorage", c === true)
											}
										/>
										<Label htmlFor="userStorage">
											{t("userscopedStorage", "User-scoped storage")}
										</Label>
									</div>
								</div>

								<Separator />

								<div className="space-y-4">
									<h4 className="font-medium">
										{t("additionalCapabilities", "Additional Capabilities")}
									</h4>
									<div className="grid grid-cols-2 gap-2">
										<div className="flex items-center space-x-2">
											<Checkbox
												id="variables"
												checked={formData.variables}
												onCheckedChange={(c) =>
													updateField("variables", c === true)
												}
											/>
											<Label htmlFor="variables">
												{t("variables", "Variables")}
											</Label>
										</div>
										<div className="flex items-center space-x-2">
											<Checkbox
												id="cache"
												checked={formData.cache}
												onCheckedChange={(c) =>
													updateField("cache", c === true)
												}
											/>
											<Label htmlFor="cache">{t("cache", "Cache")}</Label>
										</div>
										<div className="flex items-center space-x-2">
											<Checkbox
												id="streaming"
												checked={formData.streaming}
												onCheckedChange={(c) =>
													updateField("streaming", c === true)
												}
											/>
											<Label htmlFor="streaming">
												{t("streaming", "Streaming")}
											</Label>
										</div>
										<div className="flex items-center space-x-2">
											<Checkbox
												id="a2ui"
												checked={formData.a2ui}
												onCheckedChange={(c) => updateField("a2ui", c === true)}
											/>
											<Label htmlFor="a2ui">{`A2UI`}</Label>
										</div>
										<div className="flex items-center space-x-2">
											<Checkbox
												id="models"
												checked={formData.models}
												onCheckedChange={(c) =>
													updateField("models", c === true)
												}
											/>
											<Label htmlFor="models">
												{t("modelsLlm", "Models / LLM")}
											</Label>
										</div>
									</div>
								</div>
							</CardContent>
						</>
					)}

					{step === "review" && (
						<>
							<CardHeader>
								<CardTitle className="text-lg">
									{t("reviewSubmit", "Review & Submit")}
								</CardTitle>
								<CardDescription>
									{t(
										"reviewYourPackageDetailsBeforeSubmittingForReview",
										"Review your package details before submitting for review",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent className="space-y-6">
								<div className="rounded-lg border p-4 space-y-3">
									<div className="flex items-center gap-2">
										<Package className="h-5 w-5" />
										<span className="font-semibold">{formData.name}</span>
										<Badge variant="outline">{`v${formData.version}`}</Badge>
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
										{t("permissionsSummary", "Permissions Summary")}
									</h4>
									<div className="flex flex-wrap gap-1">
										<Badge variant="outline">
											{t("memoryMemorytier", "Memory: {{memoryTier}}", {
												memoryTier: formData.memoryTier,
											})}
										</Badge>
										<Badge variant="outline">
											{t("timeoutTimeouttier", "Timeout: {{timeoutTier}}", {
												timeoutTier: formData.timeoutTier,
											})}
										</Badge>
										{formData.httpEnabled && (
											<Badge variant="outline">{`HTTP`}</Badge>
										)}
										{formData.websocketEnabled && (
											<Badge variant="outline">
												{t("websocket", "WebSocket")}
											</Badge>
										)}
										{formData.nodeStorage && (
											<Badge variant="outline">
												{t("nodeStorage", "Node Storage")}
											</Badge>
										)}
										{formData.userStorage && (
											<Badge variant="outline">
												{t("userStorage", "User Storage")}
											</Badge>
										)}
										{formData.variables && (
											<Badge variant="outline">
												{t("variables", "Variables")}
											</Badge>
										)}
										{formData.cache && (
											<Badge variant="outline">{t("cache", "Cache")}</Badge>
										)}
										{formData.streaming && (
											<Badge variant="outline">
												{t("streaming", "Streaming")}
											</Badge>
										)}
										{formData.a2ui && <Badge variant="outline">{`A2UI`}</Badge>}
										{formData.models && (
											<Badge variant="outline">{t("models", "Models")}</Badge>
										)}
									</div>
								</div>

								<div className="rounded-lg bg-yellow-500/10 border border-yellow-500/20 p-4">
									<div className="flex items-start gap-2">
										<AlertTriangle className="h-5 w-5 text-yellow-500 mt-0.5" />
										<div>
											<h4 className="font-medium text-yellow-500">
												{t("adminReviewRequired", "Admin Review Required")}
											</h4>
											<p className="text-sm text-muted-foreground mt-1">
												{t(
													"yourPackageWillBeSubmittedForReviewAnAdminWillReviewTheCodeAndPermissionsBeforeItBecomesAvailableInThePublicRegistry",
													"Your package will be submitted for review. An admin will review the code and permissions before it becomes available in the public registry.",
												)}
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
							{t("back", "Back")}
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
										{t("submitForReview", "Submit for Review")}
									</>
								)}
							</Button>
						) : (
							<Button onClick={nextStep} disabled={!canProceed()}>
								{t("continue", "Continue")}
								<ChevronRight className="ml-2 h-4 w-4" />
							</Button>
						)}
					</div>
				</Card>
			</div>
		</main>
	);
}
