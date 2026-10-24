"use client";

import {
	Bit,
	Button,
	type IHub,
	type UseQueryResult,
	isHostableLlmModel,
	isLocalLlmModel,
	useBackend,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import {
	type IBit,
	IBitTypes,
} from "@flow-like/flow-like-ui/lib/schema/bit/bit";
import { humanFileSize } from "@flow-like/flow-like-ui/lib/utils";
import type { ISettingsProfile } from "@flow-like/flow-like-ui/types";
import { Trans, useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import {
	ArrowRight,
	Check,
	Cloud,
	Download,
	Layers3,
	Loader2,
	LogIn,
	Monitor,
	Plus,
	RefreshCw,
	RotateCcw,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import {
	type CSSProperties,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { useTauriInvoke } from "../../components/useInvoke";
import {
	type OnlineProfile,
	getDefaultApiBase,
	toLocalProfile,
} from "../../lib/profile-sync";
import { OnboardingFlow } from "./onboarding-flow";
import styles from "./onboarding.module.css";

type ProfileEntry = {
	profile: ISettingsProfile;
	bits: IBit[];
	hasLocalModels: boolean;
	hasUnsupportedModels: boolean;
	requiresSignIn: boolean;
};

const LLM_TYPES = new Set<IBitTypes>([IBitTypes.Llm, IBitTypes.Vlm]);
const usesLocalArtifacts = (bit: IBit): boolean =>
	(bit.size ?? 0) > 0 ||
	Boolean(bit.download_link) ||
	(bit.dependencies?.length ?? 0) > 0;

export default function Onboarding() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const queryClient = useQueryClient();
	const { canHostLlamaCPP, canHostMLX } = backend.capabilities();
	const isAuthenticated = Boolean(auth?.isAuthenticated);
	const accessToken = auth?.user?.access_token;
	const [activeProfiles, setActiveProfiles] = useState<string[]>([]);
	const [introRun, setIntroRun] = useState(0);
	const [size, setSize] = useState<number | null>(0);
	const [sizeFailed, setSizeFailed] = useState(false);
	const [isPullingProfiles, setIsPullingProfiles] = useState(false);
	const [restoreFailed, setRestoreFailed] = useState(false);
	const [restoreAttempt, setRestoreAttempt] = useState(0);
	const [isSigningIn, setIsSigningIn] = useState(false);
	const [signInOpened, setSignInOpened] = useState(false);
	const [signInFailed, setSignInFailed] = useState(false);
	const defaultProfiles: UseQueryResult<[[ISettingsProfile, IBit[]][], IHub]> =
		useTauriInvoke("get_default_profiles", {});

	const processedProfiles = useMemo<ProfileEntry[]>(
		() =>
			(defaultProfiles.data?.[0] ?? []).map(([profile, bits]) => {
				const models = bits.filter((bit) => LLM_TYPES.has(bit.type));
				return {
					profile,
					bits,
					hasLocalModels: models.some(
						(bit) => isLocalLlmModel(bit) || usesLocalArtifacts(bit),
					),
					hasUnsupportedModels: models.some(
						(bit) => !isHostableLlmModel(bit, { canHostLlamaCPP, canHostMLX }),
					),
					requiresSignIn:
						models.length > 0 &&
						models.every(
							(bit) => !isLocalLlmModel(bit) && !usesLocalArtifacts(bit),
						),
				};
			}),
		[defaultProfiles.data, canHostLlamaCPP, canHostMLX],
	);
	const availableProfiles = useMemo(
		() =>
			processedProfiles.filter(
				(entry) => !entry.hasUnsupportedModels && entry.profile.hub_profile.id,
			),
		[processedProfiles],
	);
	const filteredOutCount = processedProfiles.filter(
		(entry) => entry.hasUnsupportedModels,
	).length;
	const selectedProfiles = useMemo(
		() =>
			availableProfiles.filter(({ profile }) =>
				activeProfiles.includes(profile.hub_profile.id ?? ""),
			),
		[availableProfiles, activeProfiles],
	);
	const selectedRequiresSignIn = selectedProfiles.some(
		(entry) => entry.requiresSignIn,
	);
	const downloadHref = useMemo(() => {
		const params = new URLSearchParams();
		for (const { profile } of selectedProfiles) {
			if (profile.hub_profile.id)
				params.append("profiles", profile.hub_profile.id);
		}
		return selectedProfiles.length > 0
			? `/onboarding/download?${params.toString()}`
			: null;
	}, [selectedProfiles]);
	const signInBusy =
		isSigningIn || auth?.isLoading || Boolean(auth?.activeNavigator);
	const canSignIn = Boolean(
		auth?.settings?.authority &&
			auth.settings?.client_id &&
			!signInBusy &&
			!isAuthenticated,
	);

	const handleSignIn = useCallback(async () => {
		if (!canSignIn) return;
		setIsSigningIn(true);
		setSignInFailed(false);
		try {
			await auth.signinRedirect();
			setSignInOpened(true);
		} catch (error) {
			console.error("Sign-in failed:", error);
			setSignInFailed(true);
		} finally {
			setIsSigningIn(false);
		}
	}, [auth, canSignIn]);

	// The retry counter explicitly starts another restore after a failed request.
	// biome-ignore lint/correctness/useExhaustiveDependencies: restoreAttempt is the user-triggered retry signal.
	useEffect(() => {
		if (!isAuthenticated || !accessToken) {
			setIsPullingProfiles(false);
			setRestoreFailed(false);
			return;
		}
		let cancelled = false;
		const pullServerProfiles = async () => {
			setIsPullingProfiles(true);
			setRestoreFailed(false);
			try {
				const response = await tauriFetch(
					`${getDefaultApiBase()}/api/v1/profile`,
					{
						method: "GET",
						headers: {
							"Content-Type": "application/json",
							Authorization: `Bearer ${accessToken}`,
						},
					},
				);
				if (cancelled) return;
				if (!response.ok) throw new Error("Could not restore profiles");
				const allProfiles = (await response.json()) as OnlineProfile[];
				if (cancelled) return;
				if (!Array.isArray(allProfiles)) {
					throw new Error("Profile list response is not an array");
				}
				const serverProfiles = allProfiles.filter(
					(profile) =>
						isRecord(profile) &&
						typeof profile.id === "string" &&
						!profile.deleted_at,
				);
				let firstProfileId: string | null = null;
				for (const profile of serverProfiles) {
					if (cancelled) return;
					try {
						await invoke("upsert_profile", {
							profile: toLocalProfile(profile),
						});
						firstProfileId ??= profile.id;
					} catch (error) {
						console.error("Failed to restore profile:", profile.id, error);
					}
				}
				if (cancelled) return;
				if (firstProfileId) {
					await invoke("set_current_profile", { profileId: firstProfileId });
					await queryClient.invalidateQueries({
						queryKey: ["get", "profiles"],
					});
					// A deep link may have navigated away while profiles were restoring.
					if (!cancelled) router.replace("/");
				} else if (serverProfiles.length > 0) {
					throw new Error("No profiles could be restored");
				}
			} catch (error) {
				console.warn("Failed to pull server profiles:", error);
				if (!cancelled) setRestoreFailed(true);
			} finally {
				if (!cancelled) setIsPullingProfiles(false);
			}
		};
		void pullServerProfiles();
		return () => {
			cancelled = true;
		};
	}, [isAuthenticated, accessToken, queryClient, router, restoreAttempt]);

	useEffect(() => {
		let cancelled = false;
		setSizeFailed(false);
		if (selectedProfiles.length === 0) {
			setSize(0);
			return;
		}
		setSize(null);
		const uniqueBits = new Map<string, IBit>();
		for (const { bits } of selectedProfiles) {
			for (const bit of bits) uniqueBits.set(bit.id, bit);
		}
		void Promise.all(
			Array.from(uniqueBits.values()).map((bit) => {
				const instance = Bit.fromObject(bit);
				instance.setBackend(backend);
				return backend.bitState.getBitSize(instance.toObject());
			}),
		)
			.then((sizes) => {
				if (!cancelled)
					setSize(sizes.reduce((total, bytes) => total + bytes, 0));
			})
			.catch(() => {
				if (!cancelled) setSizeFailed(true);
			});
		return () => {
			cancelled = true;
		};
	}, [selectedProfiles, backend]);

	const toggleProfile = (id: string) => {
		setActiveProfiles((previous) =>
			previous.includes(id)
				? previous.filter((selected) => selected !== id)
				: [...previous, id],
		);
	};
	const signInLabel = signInBusy
		? t("onboarding.openingSignIn", "Connecting…")
		: signInOpened
			? t("onboarding.openSignInAgain", "Open sign-in again")
			: t("signIn", "Sign in");

	return (
		<main className={styles.page}>
			<header className={styles.header}>
				<div className={styles.brand}>
					<img src="/app-logo.webp" width={34} height={34} alt="" />
					<span>Flow-Like</span>
				</div>
				{!isAuthenticated ? (
					<div className={styles.returning}>
						<div>
							<strong>
								{t("onboarding.returning", "Already use Flow-Like?")}
							</strong>
							<span>
								{t(
									"onboarding.restoreHint",
									"Bring your profiles to this device.",
								)}
							</span>
						</div>
						<Button
							variant="outline"
							className={styles.signIn}
							disabled={!canSignIn}
							onClick={handleSignIn}
						>
							{signInBusy ? (
								<Loader2 className="animate-spin" aria-hidden="true" />
							) : (
								<LogIn aria-hidden="true" />
							)}
							{signInLabel}
						</Button>
					</div>
				) : (
					<span className={styles.signedIn}>
						<Check size={15} aria-hidden="true" />
						{t("onboarding.signedIn", "Signed in")}
					</span>
				)}
			</header>

			<section className={styles.hero} aria-labelledby="welcome-heading">
				<div className={styles.heroCopy}>
					<p className={styles.eyebrow}>
						{t("onboarding.welcome", "Welcome to your new workspace")}
					</p>
					<h1 id="welcome-heading">
						<Trans ns="common" i18nKey="onboarding.heroTitle">
							Make your <span className={styles.heroAccent}>ideas flow.</span>
						</Trans>
					</h1>
					<p className={styles.intro}>
						{t(
							"onboarding.intro",
							"Build workflows with your tools and AI, all in one workspace.",
						)}
					</p>
				</div>
				<div className={styles.heroArt}>
					<OnboardingFlow key={introRun} />
				</div>
				<button
					type="button"
					className={styles.replay}
					onClick={() => setIntroRun((run) => run + 1)}
					aria-label={t("onboarding.replayIntro", "Replay welcome animation")}
					title={t("onboarding.replayIntro", "Replay welcome animation")}
				>
					<RotateCcw size={14} aria-hidden="true" />
				</button>
			</section>

			{!isAuthenticated && (signInFailed || signInOpened) && (
				<div className={styles.notice} role={signInFailed ? "alert" : "status"}>
					<LogIn size={18} aria-hidden="true" />
					<p>
						{signInFailed
							? t(
									"onboarding.signInFailed",
									"Sign-in could not open. Try again, or choose a profile to get started.",
								)
							: t(
									"onboarding.finishSignIn",
									"Finish signing in in your browser. Your saved profiles will appear here automatically.",
								)}
					</p>
				</div>
			)}

			{isPullingProfiles ? (
				<div className={styles.restore} aria-live="polite">
					<Loader2 className="animate-spin" size={28} aria-hidden="true" />
					<h2>{t("restoringYourProfiles", "Restoring your profiles…")}</h2>
					<p>
						{t(
							"onboarding.restoringHint",
							"Getting your workspace ready on this device.",
						)}
					</p>
				</div>
			) : (
				<>
					{restoreFailed && (
						<div className={styles.notice} role="alert">
							<p>
								{t(
									"onboarding.restoreFailed",
									"We couldn't restore your saved profiles. Try again, or start with a profile below.",
								)}
							</p>
							<Button
								variant="outline"
								onClick={() => setRestoreAttempt((attempt) => attempt + 1)}
							>
								<RefreshCw aria-hidden="true" />
								{t("tryAgain", "Try again")}
							</Button>
						</div>
					)}
					<section
						className={styles.profiles}
						aria-labelledby="profiles-heading"
						aria-busy={defaultProfiles.isLoading}
					>
						<div className={styles.sectionHeading}>
							<div>
								<p className={styles.eyebrow}>
									{t("onboarding.makeItYours", "Make it yours")}
								</p>
								<h2 id="profiles-heading">
									{t(
										"onboarding.chooseProfiles",
										"Choose your starting profiles",
									)}
								</h2>
								<p>
									{t(
										"onboarding.profilesHint",
										"Profiles bring together your tools and AI models. Choose one or more to get started.",
									)}
								</p>
							</div>
							<span className={styles.changeLater}>
								{t("onboarding.changeLater", "You can change these anytime")}
							</span>
						</div>

						{defaultProfiles.isLoading ? (
							<output className={styles.empty}>
								<Loader2 className="animate-spin" aria-hidden="true" />
								{t(
									"onboarding.loadingProfiles",
									"Finding your starting profiles…",
								)}
							</output>
						) : defaultProfiles.isError ? (
							<div className={styles.empty} role="alert">
								<p>
									{t(
										"onboarding.loadFailed",
										"We couldn't load the starting profiles. Check your connection and try again.",
									)}
								</p>
								<Button
									variant="outline"
									disabled={defaultProfiles.isFetching}
									onClick={() => {
										void defaultProfiles.refetch();
									}}
								>
									<RefreshCw aria-hidden="true" />
									{t("tryAgain", "Try again")}
								</Button>
							</div>
						) : availableProfiles.length === 0 ? (
							<div className={styles.empty}>
								<Layers3 aria-hidden="true" />
								<p>
									{t(
										"onboarding.noProfiles",
										"No compatible profiles are available for this device yet. Try refreshing, or sign in to restore your own.",
									)}
								</p>
								<Button
									variant="outline"
									disabled={defaultProfiles.isFetching}
									onClick={() => {
										void defaultProfiles.refetch();
									}}
								>
									{t("refresh", "Refresh")}
								</Button>
							</div>
						) : (
							<div className={styles.grid}>
								{availableProfiles.map((entry, index) => (
									<ProfileCard
										key={entry.profile.hub_profile.id}
										entry={entry}
										index={index}
										active={activeProfiles.includes(
											entry.profile.hub_profile.id ?? "",
										)}
										isAuthenticated={isAuthenticated}
										onToggle={() =>
											toggleProfile(entry.profile.hub_profile.id ?? "")
										}
									/>
								))}
							</div>
						)}
						{filteredOutCount > 0 && (
							<p className={styles.compatibility}>
								<Monitor size={14} aria-hidden="true" />
								{t(
									"onboarding.compatibility",
									"Profiles with local models that this device cannot run are hidden.",
								)}
							</p>
						)}
					</section>

					<footer
						className={styles.footer}
						data-ready={selectedProfiles.length > 0}
					>
						<div
							className={styles.selection}
							aria-live="polite"
							aria-atomic="true"
						>
							{selectedProfiles.length > 0 ? (
								<span className={styles.selectedAvatars} aria-hidden="true">
									{selectedProfiles.slice(0, 3).map(({ profile }) => (
										<span key={profile.hub_profile.id}>
											<img
												src={profile.hub_profile.thumbnail || "/app-logo.webp"}
												alt=""
												onError={(event) => {
													event.currentTarget.style.visibility = "hidden";
												}}
											/>
										</span>
									))}
								</span>
							) : (
								<span className={styles.selectionIcon}>
									<Layers3 size={20} aria-hidden="true" />
								</span>
							)}
							<div>
								<strong>
									{selectedProfiles.length > 0
										? t("onboarding.selectedProfiles", {
												defaultValue_one: "{{count}} profile selected",
												defaultValue_other: "{{count}} profiles selected",
												count: selectedProfiles.length,
											})
										: t("onboarding.selectToStart", "No profiles selected")}
								</strong>
								<p>
									{selectedProfiles.length === 0
										? t(
												"onboarding.selectionHint",
												"Choose one or more to continue.",
											)
										: sizeFailed
											? t(
													"onboarding.sizeUnavailable",
													"Download size unavailable",
												)
											: size === null
												? t(
														"onboarding.calculatingSize",
														"Calculating download size…",
													)
												: size === 0
													? t(
															"onboarding.noDownload",
															"No model download needed",
														)
													: t(
															"onboarding.downloadSize",
															"{{size}} to download",
															{ size: humanFileSize(size) },
														)}
								</p>
							</div>
						</div>
						<div className={styles.continueArea}>
							{selectedRequiresSignIn && !isAuthenticated && (
								<span className={styles.hostedHint}>
									{t(
										"onboarding.hostedHint",
										"Hosted models need sign-in. You can do that later.",
									)}
								</span>
							)}
							<Button
								className={styles.continueButton}
								disabled={!downloadHref}
								asChild={Boolean(downloadHref)}
							>
								{downloadHref ? (
									<Link href={downloadHref}>
										{t("continue", "Continue")}
										<ArrowRight aria-hidden="true" />
									</Link>
								) : (
									<>
										{t("continue", "Continue")}
										<ArrowRight aria-hidden="true" />
									</>
								)}
							</Button>
						</div>
					</footer>
				</>
			)}
		</main>
	);
}

function ProfileCard({
	entry,
	index,
	active,
	isAuthenticated,
	onToggle,
}: {
	entry: ProfileEntry;
	index: number;
	active: boolean;
	isAuthenticated: boolean;
	onToggle: () => void;
}) {
	const { t } = useTranslation("common");
	const {
		profile: { hub_profile: profile },
		bits,
		hasLocalModels,
		requiresSignIn,
	} = entry;
	const [imageFailed, setImageFailed] = useState(false);
	const cardId = useId();
	return (
		<button
			type="button"
			className={styles.profileCard}
			style={{ "--card-order": Math.min(index, 7) } as CSSProperties}
			aria-pressed={active}
			onClick={onToggle}
			aria-labelledby={`${cardId}-name`}
			aria-describedby={`${cardId}-description ${cardId}-details${hasLocalModels || requiresSignIn ? ` ${cardId}-requirement` : ""}`}
		>
			<div className={styles.thumbnail}>
				{profile.thumbnail && !imageFailed ? (
					<img
						src={profile.thumbnail}
						alt=""
						loading="lazy"
						decoding="async"
						onError={() => setImageFailed(true)}
					/>
				) : (
					<div className={styles.thumbnailFallback}>
						<Layers3 size={40} strokeWidth={1} aria-hidden="true" />
					</div>
				)}
				<span className={styles.selectIndicator} aria-hidden="true">
					{active ? <Check size={14} strokeWidth={3} /> : <Plus size={14} />}
				</span>
				<span className={styles.profileType}>
					{hasLocalModels ? (
						<>
							<Monitor size={12} aria-hidden="true" />
							{t("onboarding.local", "Local models")}
						</>
					) : requiresSignIn ? (
						<>
							<Cloud size={12} aria-hidden="true" />
							{t("onboarding.hosted", "Hosted models")}
						</>
					) : (
						<>
							<Layers3 size={12} aria-hidden="true" />
							{t("onboarding.starter", "Starter profile")}
						</>
					)}
				</span>
			</div>
			<div className={styles.cardBody}>
				<div className={styles.cardTitle}>
					<h3 id={`${cardId}-name`}>{profile.name}</h3>
				</div>
				<p id={`${cardId}-description`} className={styles.description}>
					{profile.description}
				</p>
				<div
					id={`${cardId}-details`}
					className={styles.cardMeta}
					title={bits.map((bit) => bit.meta?.en?.name ?? bit.id).join(", ")}
				>
					<span className={styles.bitIcons} aria-hidden="true">
						{bits.slice(0, 3).map((bit) => (
							<img
								key={bit.id}
								src={bit.meta?.en?.icon || "/app-logo.webp"}
								alt=""
								width={22}
								height={22}
								onError={(event) => {
									event.currentTarget.style.visibility = "hidden";
								}}
							/>
						))}
					</span>
					<span>
						{t("onboarding.includedTools", {
							defaultValue_one: "{{count}} included tool",
							defaultValue_other: "{{count}} included tools",
							count: bits.length,
						})}
					</span>
					<span className={styles.cardAction} aria-hidden="true">
						{active ? (
							<>
								<Check size={12} />
								{t("onboarding.profileSelected", "Selected")}
							</>
						) : (
							<>
								<Plus size={12} />
								{t("onboarding.add", "Add")}
							</>
						)}
					</span>
				</div>
				{(hasLocalModels || requiresSignIn) && (
					<span id={`${cardId}-requirement`} className={styles.cardRequirement}>
						{hasLocalModels ? (
							<>
								<Download size={12} aria-hidden="true" />
								{t("localModelDownload", "Local model download")}
							</>
						) : (
							<>
								<LogIn size={12} aria-hidden="true" />
								{isAuthenticated
									? t("hostedModelReady", "Hosted model ready")
									: t("requiresSignIn", "Requires sign in")}
							</>
						)}
					</span>
				)}
			</div>
		</button>
	);
}
