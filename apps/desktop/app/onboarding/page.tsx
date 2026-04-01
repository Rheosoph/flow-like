"use client";
import { invoke } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import type { IHub, UseQueryResult } from "@tm9657/flow-like-ui";
import { Bit, Button, useBackend } from "@tm9657/flow-like-ui";
import {
	Alert,
	AlertDescription,
	AlertTitle,
} from "@tm9657/flow-like-ui/components/ui/alert";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
} from "@tm9657/flow-like-ui/components/ui/avatar";
import { Badge } from "@tm9657/flow-like-ui/components/ui/badge";
import { BitHover } from "@tm9657/flow-like-ui/components/ui/bit-hover";
import type { IBit } from "@tm9657/flow-like-ui/lib/schema/bit/bit";
import { IBitTypes } from "@tm9657/flow-like-ui/lib/schema/bit/bit";
import { humanFileSize } from "@tm9657/flow-like-ui/lib/utils";
import type { ISettingsProfile } from "@tm9657/flow-like-ui/types";
import { ArrowBigRight, CloudDownload, Loader2, LogIn } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import {
	useInvalidateTauriInvoke,
	useTauriInvoke,
} from "../../components/useInvoke";
import {
	type OnlineProfile,
	getDefaultApiBase,
	toLocalProfile,
} from "../../lib/profile-sync";

type ProfileEntry = {
	profile: ISettingsProfile;
	bits: IBit[];
	hasLocalModels: boolean;
	requiresSignIn: boolean;
};

const LLM_TYPES = new Set<IBitTypes>([IBitTypes.Llm, IBitTypes.Vlm]);
const LOCAL_PROVIDERS = new Set(["local", "llama.cpp", "llamacpp"]);

const usesLocalWeights = (bit: IBit): boolean => (bit.size ?? 0) > 0;

const getProviderName = (bit: IBit): string => {
	const parameters =
		typeof bit.parameters === "object" && bit.parameters !== null
			? (bit.parameters as {
					provider?: {
						provider_name?: string | null;
					};
				})
			: undefined;
	const providerName = parameters?.provider?.provider_name;
	return typeof providerName === "string" ? providerName.toLowerCase() : "";
};

const requiresHostedSignIn = (bit: IBit): boolean => {
	if (usesLocalWeights(bit)) return false;
	const providerName = getProviderName(bit);
	if (!providerName) return true;
	return !LOCAL_PROVIDERS.has(providerName);
};

// Module-level flag — survives component remounts within the same SPA session,
// unlike useRef which resets every time the route re-mounts the component.
let hasInitiatedProfilePull = false;

export default function Onboarding() {
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const invalidate = useInvalidateTauriInvoke();
	const canHostModels = backend.capabilities().canHostLlamaCPP;
	const isAuthenticated = Boolean(auth?.isAuthenticated);
	const [profiles, setProfiles] = useState<[ISettingsProfile, IBit[]][]>([]);
	const [route, setRoute] = useState("");
	const [totalSize, setTotalSize] = useState(0);
	const [isPullingProfiles, setIsPullingProfiles] = useState(false);
	const defaultProfiles: UseQueryResult<[[ISettingsProfile, IBit[]][], IHub]> =
		useTauriInvoke("get_default_profiles", {});
	const [activeProfiles, setActiveProfiles] = useState<string[]>([]);

	const processedProfiles = useMemo<ProfileEntry[]>(
		() =>
			profiles.map(([profile, bits]) => {
				const llmLikeBits = bits.filter((bit) => LLM_TYPES.has(bit.type));
				const hasLocalModels = llmLikeBits.some(usesLocalWeights);
				const requiresSignIn =
					llmLikeBits.length > 0 && llmLikeBits.every(requiresHostedSignIn);

				return {
					profile,
					bits,
					hasLocalModels,
					requiresSignIn,
				};
			}),
		[profiles],
	);

	const availableProfiles = useMemo<ProfileEntry[]>(
		() =>
			canHostModels
				? processedProfiles
				: processedProfiles.filter((entry) => !entry.hasLocalModels),
		[processedProfiles, canHostModels],
	);

	const filteredOutCount = processedProfiles.length - availableProfiles.length;
	const signInRequiredProfiles = useMemo(
		() => availableProfiles.filter((entry) => entry.requiresSignIn),
		[availableProfiles],
	);
	const availableProfileIds = useMemo(
		() =>
			availableProfiles
				.map(({ profile }) => profile.hub_profile.id)
				.filter((id): id is string => Boolean(id)),
		[availableProfiles],
	);
	const hasSignInProfiles = signInRequiredProfiles.length > 0;
	const noProfilesAvailable = availableProfiles.length === 0;
	const canTriggerSignIn = Boolean(auth?.signinRedirect) && !isAuthenticated;
	const selectedRequiresSignIn = useMemo(
		() =>
			signInRequiredProfiles.some(({ profile }) => {
				const id = profile.hub_profile.id;
				return id ? activeProfiles.includes(id) : false;
			}),
		[signInRequiredProfiles, activeProfiles],
	);
	const downloadHref =
		route.length > 0 ? `/onboarding/download?${route}` : null;
	const showLocalModelAlert = !canHostModels && filteredOutCount > 0;

	const handleSignIn = useCallback(async () => {
		if (!auth?.signinRedirect) return;
		try {
			await auth.signinRedirect();
		} catch (error) {
			console.error("signinRedirect failed:", error);
		}
	}, [auth]);

	useEffect(() => {
		const accessToken = auth?.user?.access_token;
		if (!isAuthenticated || !accessToken) return;
		if (hasInitiatedProfilePull) return;
		hasInitiatedProfilePull = true;

		let cancelled = false;
		const pullServerProfiles = async () => {
			setIsPullingProfiles(true);
			try {
				const apiBase = getDefaultApiBase();
				const url = `${apiBase}/api/v1/profile`;
				const response = await tauriFetch(url, {
					method: "GET",
					headers: {
						"Content-Type": "application/json",
						Authorization: `Bearer ${accessToken}`,
					},
				});

				if (!response.ok || cancelled) {
					if (!response.ok) hasInitiatedProfilePull = false;
					if (!cancelled) setIsPullingProfiles(false);
					return;
				}

				const allProfiles = (await response.json()) as OnlineProfile[];
				if (cancelled) return;
				const serverProfiles = allProfiles.filter((p) => !p.deleted_at);

				if (serverProfiles.length > 0) {
					let firstProfileId: string | null = null;
					for (const op of serverProfiles) {
						try {
							const localProfile = toLocalProfile(op);
							await invoke("upsert_profile", {
								profile: localProfile,
							});
							firstProfileId ??= op.id;
						} catch (error) {
							console.error("Failed to create profile:", op.id, error);
						}
					}

					if (firstProfileId && !cancelled) {
						await invoke("set_current_profile", {
							profileId: firstProfileId,
						});
						await invalidate("get_profiles");
						router.replace("/");
						return;
					}
				}
			} catch (error) {
				hasInitiatedProfilePull = false;
				console.warn("Failed to pull server profiles:", error);
			}
			if (!cancelled) setIsPullingProfiles(false);
		};

		pullServerProfiles();
		return () => {
			cancelled = true;
		};
	}, [isAuthenticated, auth?.user?.access_token, router]);

	const handleToggleProfile = useCallback((profileId: string) => {
		setActiveProfiles((previous) =>
			previous.includes(profileId)
				? previous.filter((id) => id !== profileId)
				: [...previous, profileId],
		);
	}, []);

	useEffect(() => {
		setActiveProfiles((previous) => {
			if (availableProfileIds.length === 0) {
				return previous.length === 0 ? previous : [];
			}

			const allowed = new Set(availableProfileIds);
			const filtered = previous.filter((id) => allowed.has(id));
			if (filtered.length === previous.length) return previous;
			return filtered;
		});
	}, [availableProfileIds]);

	const calculateSize = useCallback(async () => {
		const uniqueBits = new Map<string, Bit>();
		for (const profileId of activeProfiles) {
			const profileEntry = profiles.find(
				([profile]) => profile.hub_profile.id === profileId,
			);
			if (!profileEntry) continue;
			const [, profileBits] = profileEntry;
			for (const bit of profileBits) {
				const bitInstance = Bit.fromObject(bit);
				bitInstance.setBackend(backend);
				uniqueBits.set(bit.id, bitInstance);
			}
		}

		const sizes = await Promise.all(
			Array.from(uniqueBits.values()).map((bit) =>
				backend.bitState.getBitSize(bit.toObject()),
			),
		);
		setTotalSize(sizes.reduce((acc, size) => acc + size, 0));
	}, [activeProfiles, profiles, backend]);

	useEffect(() => {
		if (!backend) return;
		void calculateSize();
		if (activeProfiles.length === 0) {
			setRoute("");
			return;
		}

		const params = activeProfiles.map((id) => `profiles=${id}`).join("&");
		setRoute(params);
	}, [activeProfiles, backend, calculateSize]);

	useEffect(() => {
		if (!defaultProfiles.data) return;
		const profiles = defaultProfiles.data as [
			[ISettingsProfile, IBit[]][],
			IHub,
		];
		setProfiles(profiles[0]);
	}, [defaultProfiles.data]);

	return (
		<div className="flex flex-col items-center justify-start w-full min-h-0 px-3 py-4 sm:px-6 sm:py-6 pb-12 sm:pb-16 z-10 gap-8">
			<OnboardingIntro />
			{isPullingProfiles ? (
				<RestoreProfilesLoading />
			) : (
				<>
					{!isAuthenticated && (
						<ExistingAccountBanner onSignIn={handleSignIn} />
					)}
					<ProfilesSection
						showLocalModelAlert={showLocalModelAlert}
						filteredOutCount={filteredOutCount}
						hasSignInProfiles={hasSignInProfiles}
						isAuthenticated={isAuthenticated}
						noProfilesAvailable={noProfilesAvailable}
						availableProfiles={availableProfiles}
						activeProfiles={activeProfiles}
						onToggleProfile={handleToggleProfile}
					/>
					<DownloadPanel
						totalSize={totalSize}
						selectedRequiresSignIn={selectedRequiresSignIn}
						hasSignInProfiles={hasSignInProfiles}
						canTriggerSignIn={canTriggerSignIn}
						isAuthenticated={isAuthenticated}
						onSignIn={handleSignIn}
						downloadHref={downloadHref}
					/>
				</>
			)}
			<br />
		</div>
	);
}

function OnboardingIntro() {
	return (
		<div className="text-center space-y-4 max-w-2xl px-1 sm:px-2">
			<div className="space-y-2">
				<h1 className="text-3xl sm:text-5xl font-bold text-foreground tracking-tight">
					Welcome to <span className="highlight">Flow-Like</span>
				</h1>
				<div className="w-24 h-1 mx-auto rounded-full bg-gradient-to-r from-primary to-primary/70" />
			</div>
			<h2 className="text-xl sm:text-2xl text-muted-foreground font-medium mt-4 sm:mt-6">
				Select your starting profile
			</h2>
			<p className="text-sm sm:text-base text-muted-foreground/80 max-w-lg mx-auto leading-relaxed">
				Choose one or more profiles that match your interests. You can always
				add, change or remove profiles later.
			</p>
		</div>
	);
}

function ExistingAccountBanner({
	onSignIn,
}: Readonly<{ onSignIn: () => void }>) {
	const handleClick = () => {
		console.log("[Onboarding] ExistingAccountBanner Sign in clicked");
		onSignIn();
	};
	return (
		<div className="w-full max-w-6xl">
			<div className="flex flex-col items-center gap-4 rounded-xl border bg-card/80 backdrop-blur-sm px-6 py-6 shadow-sm">
				<div className="text-center space-y-2">
					<h3 className="text-lg font-semibold">Already have an account?</h3>
					<p className="text-sm text-muted-foreground">
						Sign in to restore your profiles and settings from another device.
					</p>
				</div>
				<Button variant="outline" className="gap-2" onClick={handleClick}>
					<LogIn className="h-4 w-4" />
					Sign in
				</Button>
			</div>
			<div className="relative mt-8">
				<div className="absolute inset-0 flex items-center">
					<div className="w-full border-t border-border/60" />
				</div>
				<div className="relative flex justify-center text-xs uppercase">
					<span className="bg-background px-2 text-muted-foreground">
						Or choose a starter profile
					</span>
				</div>
			</div>
		</div>
	);
}

function RestoreProfilesLoading() {
	return (
		<div className="w-full max-w-6xl flex flex-col items-center gap-4 py-12">
			<Loader2 className="h-8 w-8 animate-spin text-primary" />
			<p className="text-sm text-muted-foreground">Restoring your profiles…</p>
		</div>
	);
}

function ProfilesSection({
	showLocalModelAlert,
	filteredOutCount,
	hasSignInProfiles,
	noProfilesAvailable,
	isAuthenticated,
	availableProfiles,
	activeProfiles,
	onToggleProfile,
}: Readonly<{
	showLocalModelAlert: boolean;
	filteredOutCount: number;
	hasSignInProfiles: boolean;
	noProfilesAvailable: boolean;
	isAuthenticated: boolean;
	availableProfiles: ProfileEntry[];
	activeProfiles: string[];
	onToggleProfile: (profileId: string) => void;
}>) {
	const hiddenMessage =
		filteredOutCount === 1
			? "1 profile includes a downloadable local LLM/VLM and isn't available on this device."
			: `${filteredOutCount} profiles include downloadable local LLMs/VLMs and aren't available on this device.`;

	return (
		<div className="w-full max-w-6xl space-y-4 sm:space-y-6">
			{showLocalModelAlert && (
				<Alert className="bg-card/80 backdrop-blur-sm border-border/60 shadow-sm">
					<AlertTitle>Local model hosting required</AlertTitle>
					<AlertDescription>{hiddenMessage}</AlertDescription>
				</Alert>
			)}
			{hasSignInProfiles && (
				<Alert className="bg-muted/60 border-border/60 backdrop-blur-sm">
					<AlertTitle>
						{isAuthenticated
							? "Hosted models unlocked"
							: "Hosted models need a sign-in"}
					</AlertTitle>
					<AlertDescription>
						{isAuthenticated
							? "You're already signed in—hosted profiles will connect automatically when you download."
							: "Profiles marked “Requires sign in” rely on hosted LLMs or VLMs. Continue now and sign in whenever you're ready."}
					</AlertDescription>
				</Alert>
			)}
			{noProfilesAvailable ? (
				<div className="rounded-2xl border border-dashed border-border/60 bg-muted/40 px-6 py-12 text-center text-sm text-muted-foreground">
					No compatible profiles are available for your current setup. Enable
					local model hosting or sign in to unlock more starter profiles.
				</div>
			) : (
				<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
					{availableProfiles.map(
						({ profile, bits, hasLocalModels, requiresSignIn }) => {
							const profileId = profile.hub_profile.id;
							if (!profileId) return null;
							const isActive = activeProfiles.includes(profileId);
							return (
								<PreviewCard
									key={profileId}
									bits={bits}
									profile={profile}
									active={isActive}
									hasLocalModels={hasLocalModels}
									requiresSignIn={requiresSignIn}
									isAuthenticated={isAuthenticated}
									onClick={() => onToggleProfile(profileId)}
								/>
							);
						},
					)}
				</div>
			)}
		</div>
	);
}

function DownloadPanel({
	totalSize,
	selectedRequiresSignIn,
	hasSignInProfiles,
	canTriggerSignIn,
	isAuthenticated,
	onSignIn,
	downloadHref,
}: Readonly<{
	totalSize: number;
	selectedRequiresSignIn: boolean;
	hasSignInProfiles: boolean;
	canTriggerSignIn: boolean;
	isAuthenticated: boolean;
	onSignIn: () => Promise<void> | void;
	downloadHref: string | null;
}>) {
	const downloadDisabled = !downloadHref;

	return (
		<div className="w-full max-w-6xl pb-8">
			<div className="flex items-center justify-between gap-4 rounded-xl border bg-card/80 backdrop-blur-sm px-4 py-3 shadow-sm">
				<div className="flex items-center gap-3">
					<CloudDownload className="h-5 w-5 text-primary" />
					<div className="flex items-baseline gap-2">
						<span className="text-lg font-medium">
							{humanFileSize(totalSize)}
						</span>
						{hasSignInProfiles && (
							<Badge
								variant="outline"
								className={`text-[0.65rem] ${
									isAuthenticated
										? "bg-emerald-500/10 text-emerald-500 border-emerald-500/40"
										: "text-amber-600"
								}`}
							>
								{isAuthenticated ? "Ready" : "Sign-in needed"}
							</Badge>
						)}
					</div>
				</div>

				<div className="flex items-center gap-2">
					{hasSignInProfiles && canTriggerSignIn && (
						<Button variant="outline" size="sm" onClick={onSignIn}>
							<LogIn className="h-4 w-4" />
						</Button>
					)}
					<Button
						className={`gap-2 ${
							downloadDisabled
								? "bg-muted text-muted-foreground cursor-not-allowed"
								: "bg-primary text-primary-foreground hover:bg-primary/90"
						}`}
						disabled={downloadDisabled}
						asChild={Boolean(downloadHref)}
					>
						{downloadHref ? (
							<a href={downloadHref}>
								<ArrowBigRight className="w-4 h-4" /> Download
							</a>
						) : (
							<>
								<ArrowBigRight className="w-4 h-4" /> Select profiles
							</>
						)}
					</Button>
				</div>
			</div>
		</div>
	);
}

function PreviewCard({
	profile,
	bits,
	onClick,
	active,
	hasLocalModels = false,
	requiresSignIn = false,
	isAuthenticated = false,
}: Readonly<{
	bits: IBit[];
	profile: ISettingsProfile;
	onClick?: () => void;
	active?: boolean;
	hasLocalModels?: boolean;
	requiresSignIn?: boolean;
	isAuthenticated?: boolean;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			aria-pressed={active}
			className={`group relative flex flex-col w-full max-w-full sm:w-64 transition-all duration-500 rounded-2xl z-20 border-2 hover:shadow-2xl transform hover:-translate-y-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40 ${
				active
					? "border-primary bg-primary/5 shadow-lg shadow-primary/20 scale-105"
					: "border-border bg-card/50 backdrop-blur-sm hover:border-primary/50 hover:bg-card/80"
			}`}
		>
			{/* Selection Indicator */}
			{active && (
				<div className="absolute top-4 right-4 z-10">
					<div className="w-6 h-6 bg-primary rounded-full flex items-center justify-center shadow-lg animate-in zoom-in-50 duration-300">
						<div className="w-2 h-2 bg-primary-foreground rounded-full" />
					</div>
				</div>
			)}

			{/* Thumbnail Section - 16:9 aspect ratio */}
			<div className="relative w-full aspect-video overflow-hidden rounded-t-xl">
				<img
					className="absolute rounded-t-xl inset-0 w-full h-full object-cover transition-transform duration-700 group-hover:scale-110"
					src={profile.hub_profile.thumbnail ?? "/placeholder-thumbnail.webp"}
					width={1280}
					height={640}
					alt={profile.hub_profile.name}
					loading="lazy"
					decoding="async"
					fetchPriority="low"
				/>
				<div
					className={`absolute inset-0 transition-all duration-300 ${
						active ? "bg-primary/20" : "bg-black/20 group-hover:bg-primary/10"
					}`}
				/>

				{/* Gradient Overlay */}
				<div className="absolute inset-0 bg-gradient-to-t from-black/40 via-transparent to-transparent" />
			</div>

			{/* Content Section */}
			<div className="flex flex-col p-4 space-y-3 flex-1 max-w-full overflow-hidden">
				<h3 className="font-semibold text-foreground text-left leading-tight truncate line-clamp-1 max-w-full overflow-hidden">
					{profile.hub_profile.name}
				</h3>

				<p className="text-sm text-muted-foreground text-left line-clamp-2 leading-relaxed max-w-full overflow-hidden">
					{profile.hub_profile.description}
				</p>

				{(hasLocalModels || requiresSignIn) && (
					<div className="flex flex-wrap items-center gap-2 pt-1">
						{hasLocalModels && (
							<Badge
								variant={active ? "default" : "secondary"}
								className="text-[0.65rem] uppercase tracking-wide"
							>
								Local model download
							</Badge>
						)}
						{requiresSignIn && (
							<Badge
								variant="outline"
								className={`text-[0.65rem] uppercase tracking-wide ${
									isAuthenticated
										? "bg-emerald-500/10 text-emerald-500 border-emerald-500/40"
										: ""
								}`}
							>
								{isAuthenticated ? "Hosted model ready" : "Requires sign in"}
							</Badge>
						)}
					</div>
				)}

				{/* Bits Preview */}
				<div className="flex flex-row flex-wrap gap-1.5 pt-2">
					{bits.slice(0, 6).map((bit) => (
						<BitHover bit={bit} key={bit.id}>
							<Avatar className="border bg-background/90 w-7 h-7 transition-transform duration-200 hover:scale-110">
								<AvatarImage
									className="p-0.5"
									src={bit.meta?.en?.icon ?? "/app-logo.webp"}
								/>
								<AvatarFallback className="text-xs">NA</AvatarFallback>
							</Avatar>
						</BitHover>
					))}
					{bits.length > 6 && (
						<div className="w-7 h-7 rounded-full bg-muted flex items-center justify-center">
							<span className="text-xs text-muted-foreground">
								+{bits.length - 6}
							</span>
						</div>
					)}
				</div>
			</div>
		</button>
	);
}
