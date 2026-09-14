"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	Building2,
	ChevronDown,
	Loader2,
	Lock,
	Mail,
	ShieldCheck,
	Sparkles,
	Zap,
} from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { useFeatures } from "../../hooks/use-features";
import { useHub } from "../../hooks/use-hub";
import { useInvoke } from "../../hooks/use-invoke";
import { PLAN_LIMIT_EVENT } from "../../lib/api-error";
import { openExternalUrl } from "../../lib/open-external";
import { isTauri } from "../../lib/platform";
import {
	formatQuota,
	formatQuotaDate,
	quotaBelongsToAnotherPayer,
	quotaLabels,
	supportsHostedModel,
} from "../../lib/quota";
import type { QuotaDetail } from "../../lib/quota";
import { useBackend } from "../../state/backend-state";
import type {
	IConversionInfo,
	ITierInfo,
} from "../../state/backend-state/user-state";
import {
	type UpgradeReason,
	handleUpgradeRequiredError,
	useUpgradeDialogStore,
} from "../../state/upgrade-dialog-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "../ui/dialog";
import { QuotaWarnings } from "./quota-warning";
import { ENTERPRISE_TIER, TIER_ORDER, TierCard } from "./tier-card";

const REASON_COPY: Record<UpgradeReason, { title: string; sub: string }> = {
	"project-limit": {
		title: "Your private project allowance is full",
		sub: "Choose more private cloud projects. Public-project forks, including purchases, use no project slots.",
	},
	"model-tier": {
		title: "This model needs a higher plan",
		sub: "Choose a plan with this model, or connect your own provider.",
	},
	storage: {
		title: "Your cloud storage is full",
		sub: "Add room for hosted data, remove files you no longer need, or connect your own storage.",
	},
	executions: {
		title: "You've used your monthly cloud starts",
		sub: "Choose more cloud capacity or continue locally until your allowance renews.",
	},
	concurrency: {
		title: "Your cloud execution slots are busy",
		sub: "Wait for an active workflow to finish, run locally, or choose a plan with more concurrent cloud executions.",
	},
	runtime: {
		title: "You've used your monthly cloud runtime",
		sub: "Choose more cloud runtime or continue locally until your allowance renews.",
	},
	"ai-budget": {
		title: "Your hosted AI allowance is used up",
		sub: "Choose more hosted AI usage, connect your own model, or wait for your allowance to renew.",
	},
	"ai-calls": {
		title: "You've used your hosted AI operations",
		sub: "Choose more hosted AI operations, connect your own model, or wait for renewal.",
	},
	generic: {
		title: "Choose room for your next idea",
		sub: "More cloud runtime, hosted AI and storage, with free local execution on every plan.",
	},
};

const FREE_ALTERNATIVES: Partial<Record<UpgradeReason, string>> = {
	"ai-budget":
		"Your own models use no Flow-Like AI allowance. Your provider may charge separately.",
	"ai-calls":
		"Your own models use no Flow-Like AI operations. Your provider may charge separately.",
	"model-tier": "You can use an included model or connect your own provider.",
	"project-limit":
		"Public-project forks, including purchases, use no project slots.",
	storage:
		"Remove files you no longer need to free cloud storage, or keep your data locally.",
	concurrency:
		"A cloud slot becomes available when a running workflow finishes. You can also run locally for free.",
};

export function FreeUseTips({
	reason,
	defaultOpen = false,
}: { reason?: UpgradeReason; defaultOpen?: boolean } = {}) {
	return (
		<div className="rounded-xl bg-muted/40 px-4 py-3 text-sm">
			<p className="leading-relaxed">
				{(reason && FREE_ALTERNATIVES[reason]) ??
					"Local workflows and local embeddings are always free."}
			</p>
			<details className="group mt-2" open={defaultOpen || undefined}>
				<summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
					More ways to keep building for free
					<ChevronDown className="h-3.5 w-3.5 transition-transform group-open:rotate-180" />
				</summary>
				<ul className="mt-3 list-disc pl-5 space-y-2 text-muted-foreground">
					<li>Local execution and local embeddings are always free.</li>
					<li>
						Your own models use no Flow-Like AI budget or AI operations. Your
						provider may charge separately.
					</li>
					<li>
						Studio FlowPilot can use your GitHub Copilot, Codex or Claude Code
						instance. Your existing provider limits apply.
					</li>
					<li>
						Public-project forks, including purchases, use no project slots.
						Storage and cloud usage still count.
					</li>
				</ul>
				<p className="mt-3 text-xs text-muted-foreground">
					Cloud workflows using your own models still use cloud runtime.
				</p>
			</details>
		</div>
	);
}

export function SharedPlanLimit({
	quota,
	onClose,
}: { quota: QuotaDetail; onClose: () => void }) {
	const resource = quotaLabels[quota.resource] ?? quota.resource;
	const modelAccess = quota.resource === "hosted_model_access";
	const message = modelAccess
		? `This app uses its billing owner's ${quota.plan} plan. This hosted model requires a higher plan. Choose an included model or connect your own provider.`
		: `This app uses its billing owner's ${quota.plan} allowance. ${resource}: ${formatQuota(quota.used, quota.resource)} used, ${formatQuota(quota.reserved, quota.resource)} reserved, limit ${formatQuota(quota.limit, quota.resource)}.${quota.periodEnd ? ` Renews ${formatQuotaDate(quota.periodEnd)}.` : ""}`;
	return (
		<div className="p-6 space-y-4">
			<DialogTitle>
				{modelAccess
					? "This model is not included in the app's plan"
					: "This app has reached its billing owner's plan limit"}
			</DialogTitle>
			<DialogDescription>
				{message} The billing owner can review usage, free capacity or change
				the app's plan. Changing your personal subscription will not increase
				this app's allowance.
			</DialogDescription>
			<div className="flex flex-wrap gap-2">
				<Button
					onClick={async () => {
						try {
							await navigator.clipboard.writeText(message);
							toast.success(
								"Details copied. Share them with the app's billing owner.",
							);
						} catch {
							toast.error(
								"Copy the limit details above and share them with the billing owner.",
							);
						}
					}}
				>
					Copy details for billing owner
				</Button>
				<Button variant="outline" onClick={onClose}>
					Close
				</Button>
			</div>
			<FreeUseTips reason={modelAccess ? "model-tier" : undefined} />
		</div>
	);
}

export function TrustRow() {
	const { t } = useTranslation("common");
	return (
		<div className="flex flex-wrap items-center justify-center gap-x-6 gap-y-2 text-xs text-muted-foreground">
			<span className="flex items-center gap-1.5">
				<ShieldCheck className="h-3.5 w-3.5" />
				{t("secureCheckoutViaStripe", "Secure checkout via Stripe")}
			</span>
			<span className="flex items-center gap-1.5">
				<Zap className="h-3.5 w-3.5" />
				{t("instantActivation", "Instant activation")}
			</span>
			<span className="flex items-center gap-1.5">
				<Sparkles className="h-3.5 w-3.5" />
				{t("cancelAnytime", "Cancel anytime")}
			</span>
		</div>
	);
}

function LimitCallout({ message }: Readonly<{ message: string }>) {
	return (
		<div className="flex items-start gap-2.5 rounded-lg border border-tertiary/40 bg-tertiary/10 px-3.5 py-2.5 text-sm">
			<Lock className="mt-0.5 h-4 w-4 shrink-0 text-tertiary" />
			<span>{message}</span>
		</div>
	);
}

export interface EnterpriseContact {
	name?: string;
	email?: string;
	url?: string;
	message?: string;
}

export function EnterpriseContent({
	contact,
	headline,
	triggerMessage,
	hubName,
	asDialogTitle = true,
	contactLabel,
}: Readonly<{
	contact: EnterpriseContact;
	headline?: string;
	triggerMessage?: string;
	hubName?: string;
	/** False when an outer DialogTitle already labels the dialog. */
	asDialogTitle?: boolean;
	/** CTA label override, e.g. "Contact Your Admin" on org-managed hubs. */
	contactLabel?: string;
}>) {
	const { t } = useTranslation("common");
	const message =
		contact.message ??
		t(
			"valIsManagedByYourOrganizationReachOutToVal2ToUnlockMoreCapacityForYourAccount",
			"{{val}} is managed by your organization. Reach out to {{val2}} to unlock more capacity for your account.",
			{
				val: hubName ?? "This workspace",
				val2: contact.name ?? "your administrator",
			},
		);
	const title = headline ?? "Need more from your workspace?";

	return (
		<div className="flex flex-col items-center gap-5 px-8 pb-8 pt-2 text-center">
			<div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-linear-to-br from-primary/20 to-primary/5">
				<Building2 className="h-7 w-7 text-primary" />
			</div>
			<div className="space-y-2">
				{asDialogTitle ? (
					<DialogTitle className="text-2xl font-bold tracking-tight">
						{title}
					</DialogTitle>
				) : (
					<h2 className="text-2xl font-bold tracking-tight">{title}</h2>
				)}
				{asDialogTitle ? (
					<DialogDescription className="mx-auto max-w-md text-sm text-muted-foreground">
						{message}
					</DialogDescription>
				) : (
					<p className="mx-auto max-w-md text-sm text-muted-foreground">
						{message}
					</p>
				)}
			</div>
			{triggerMessage && (
				<div className="w-full max-w-md">
					<LimitCallout message={triggerMessage} />
				</div>
			)}
			<div className="flex flex-col items-center gap-2 sm:flex-row">
				{contact.email && (
					<Button asChild>
						<a
							href={`mailto:${contact.email}`}
							target="_blank"
							rel="noreferrer external"
						>
							<Mail className="h-4 w-4" />
							{contactLabel ??
								t("contactVal", "Contact {{val}}", {
									val: contact.name ?? "us",
								})}
						</a>
					</Button>
				)}
				{contact.url && (
					<Button variant="outline" asChild>
						<a href={contact.url} target="_blank" rel="noreferrer external">
							{t("learnMore", "Learn more")}
							<ArrowRight className="h-4 w-4" />
						</a>
					</Button>
				)}
			</div>
		</div>
	);
}

export interface UpgradeDialogBodyProps {
	mode: "consumer" | "enterprise";
	contact: EnterpriseContact;
	headline: string;
	subheadline: string;
	/** Headline override for the enterprise contact card. */
	enterpriseHeadline?: string;
	triggerMessage?: string;
	hubName?: string;
	isAuthenticated: boolean;
	pricingStatus: "loading" | "error" | "ready";
	onRetryPricing?: () => void;
	upgradeTiers: [string, ITierInfo][];
	currentTier: string;
	emphasizedTier?: string;
	requiredModelTier?: string;
	enterpriseTier?: ITierInfo;
	loadingTier?: string | null;
	onUpgrade: (
		tier: string,
		priceId?: string,
		interval?: "month" | "year",
	) => Promise<void> | void;
	onManageBilling: () => Promise<void> | void;
	onClose?: () => void;
	reason?: UpgradeReason;
	quota?: QuotaDetail;
}

function tierQuotaLimit(tier: ITierInfo, resource: string): number | undefined {
	const limits: Record<string, number | undefined> = {
		cloud_runtime_ms: tier.max_runtime_ms,
		hosted_ai_cost_micros:
			tier.max_ai_cost_micros ?? tier.max_llm_cost * 10_000,
		hosted_ai_calls: tier.max_llm_calls,
		cloud_starts: tier.max_remote_executions,
		concurrent_cloud_executions: tier.max_concurrent_executions,
		storage_bytes: tier.max_total_size,
		projects: tier.max_non_visible_projects,
		project_count: tier.max_non_visible_projects,
	};
	return limits[resource];
}

function tierCanCoverQuota(tier: ITierInfo, quota?: QuotaDetail): boolean {
	if (!quota || quota.resource === "hosted_model_access") return true;
	const limit = tierQuotaLimit(tier, quota.resource);
	// Recommend capacity only when the hub exposes the relevant allowance.
	if (limit === undefined) return false;
	return (
		limit < 0 ||
		(limit > quota.limit &&
			limit >= quota.used + quota.reserved + (quota.requested ?? 0))
	);
}

function quotaSummary(quota: QuotaDetail): string {
	if (quota.resource === "hosted_model_access") {
		return "Choose a plan that includes this hosted model.";
	}
	return [
		`${formatQuota(quota.used, quota.resource)} of ${formatQuota(quota.limit, quota.resource)} used.`,
		quota.reserved > 0
			? `${formatQuota(quota.reserved, quota.resource)} pending.`
			: undefined,
		quota.periodEnd ? `Renews ${formatQuotaDate(quota.periodEnd)}.` : undefined,
	]
		.filter(Boolean)
		.join(" ");
}

function upgradeBenefit(
	quota: QuotaDetail | undefined,
	tier: ITierInfo,
): string | undefined {
	if (!quota) return undefined;
	if (quota.resource === "hosted_model_access")
		return "Includes access to this hosted model.";
	const limit = tierQuotaLimit(tier, quota.resource);
	if (limit === undefined) return undefined;
	const monthly = [
		"cloud_runtime_ms",
		"hosted_ai_cost_micros",
		"hosted_ai_calls",
		"cloud_starts",
	].includes(quota.resource);
	return `${quotaLabels[quota.resource] ?? "Allowance"}: ${formatQuota(quota.limit, quota.resource)} → ${formatQuota(limit, quota.resource)}${monthly ? " per month" : ""}.`;
}

/**
 * Presentational dialog content, shared between the globally mounted
 * GlobalUpgradeDialog (live data) and the /debug/upgrade capture route
 * (synthetic data). Must be rendered inside a Dialog/DialogContent.
 */
export function UpgradeDialogBody({
	mode,
	contact,
	headline,
	subheadline,
	enterpriseHeadline,
	triggerMessage,
	hubName,
	isAuthenticated,
	pricingStatus,
	onRetryPricing,
	upgradeTiers,
	currentTier,
	emphasizedTier,
	requiredModelTier,
	enterpriseTier,
	loadingTier,
	onUpgrade,
	onManageBilling,
	onClose,
	reason = "generic",
	quota,
}: Readonly<UpgradeDialogBodyProps>) {
	const { t } = useTranslation("common");
	const eligibleTiers = upgradeTiers.filter(
		([, tier]) =>
			supportsHostedModel(tier, requiredModelTier) &&
			tierCanCoverQuota(tier, quota),
	);
	const recommended = quota
		? eligibleTiers[0]
		: (eligibleTiers.find(([key]) => key === emphasizedTier) ??
			eligibleTiers[0]);
	const alternatives = eligibleTiers.filter(
		([key]) => key !== recommended?.[0],
	);
	const benefit = recommended
		? upgradeBenefit(quota, recommended[1])
		: undefined;
	const enterpriseUrl =
		enterpriseTier?.contact_url ??
		contact.url ??
		(contact.email ? `mailto:${contact.email}` : undefined);

	return (
		<div className="relative z-10 pt-6">
			{mode === "enterprise" ? (
				<EnterpriseContent
					contact={contact}
					headline={enterpriseHeadline}
					triggerMessage={triggerMessage}
					hubName={hubName}
					contactLabel="Contact Your Admin"
				/>
			) : (
				<div className="flex flex-col gap-4 px-5 pb-5 sm:px-6 sm:pb-6">
					<div className="space-y-2 pr-5">
						<p className="text-xs font-medium text-muted-foreground">
							{currentTier.charAt(0) + currentTier.slice(1).toLowerCase()} plan
						</p>
						<DialogTitle className="text-xl font-semibold leading-tight tracking-tight sm:text-2xl">
							{headline}
						</DialogTitle>
						<DialogDescription className="text-sm leading-relaxed text-muted-foreground">
							{quota ? quotaSummary(quota) : subheadline}
						</DialogDescription>
					</div>

					{triggerMessage && !quota && (
						<div className="w-full">
							<LimitCallout message={triggerMessage} />
						</div>
					)}

					{!isAuthenticated ? (
						<p className="py-4 text-center text-sm text-muted-foreground">
							{t(
								"signInToSeeUpgradeOptionsForYourAccount",
								"Sign in to see upgrade options for your account.",
							)}
						</p>
					) : pricingStatus === "loading" ? (
						<div className="flex items-center justify-center py-12">
							<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
						</div>
					) : pricingStatus === "error" ? (
						<div className="flex flex-col items-center gap-3 py-8 text-center">
							<p className="text-sm text-muted-foreground">
								{t(
									"couldntLoadUpgradeOptionsPleaseCheckYourConnection",
									"Couldn't load upgrade options. Please check your connection.",
								)}
							</p>
							<Button variant="outline" size="sm" onClick={onRetryPricing}>
								{t("tryAgain", "Try again")}
							</Button>
						</div>
					) : recommended ? (
						<div className="space-y-4 pt-1" data-testid="recommended-upgrade">
							{benefit && <p className="text-sm font-medium">{benefit}</p>}
							<TierCard
								tierKey={recommended[0]}
								tier={recommended[1]}
								currentTier={currentTier}
								onUpgrade={onUpgrade}
								onManageBilling={onManageBilling}
								isLoading={loadingTier === recommended[0]}
								compact
								emphasize
								showDetails={false}
								focusResource={quota?.resource}
							/>
						</div>
					) : (
						<EnterpriseContent
							contact={{
								...contact,
								message: requiredModelTier
									? "Contact us about access to this hosted model, choose an included model, or connect your own provider."
									: t(
											"contactValAboutAdditionalCapacityForThisWorkflow",
											"Contact {{val}} about additional capacity for this workflow.",
											{ val: contact.name ?? "sales" },
										),
							}}
							headline={
								requiredModelTier
									? "Ask about access to this model"
									: "Let's find the capacity you need"
							}
							triggerMessage={undefined}
							hubName={hubName}
							asDialogTitle={false}
						/>
					)}

					<FreeUseTips reason={reason} />
					{isAuthenticated &&
						pricingStatus === "ready" &&
						alternatives.length > 0 && (
							<details className="group rounded-xl border p-3">
								<summary className="flex cursor-pointer list-none items-center justify-between gap-2 text-sm font-medium focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
									See other plans
									<ChevronDown className="h-4 w-4 transition-transform group-open:rotate-180" />
								</summary>
								<div className="grid gap-4 pt-5 sm:grid-cols-2">
									{alternatives.map(([key, tier]) => (
										<TierCard
											key={key}
											tierKey={key}
											tier={tier}
											currentTier={currentTier}
											onUpgrade={onUpgrade}
											onManageBilling={onManageBilling}
											isLoading={loadingTier === key}
											compact
											emphasize={false}
											showDetails={false}
											focusResource={quota?.resource}
										/>
									))}
								</div>
							</details>
						)}
					{recommended && isAuthenticated && pricingStatus === "ready" && (
						<div className="space-y-3 text-center">
							<p className="text-xs text-muted-foreground">
								Applicable taxes are shown at checkout.
							</p>
							<div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-2">
								<Link
									href="/subscription"
									onClick={onClose}
									className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
								>
									{t("compareAllPlans", "Compare all plans")}
								</Link>
								{enterpriseTier && enterpriseUrl && (
									<a
										className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
										href={enterpriseUrl}
										target="_blank"
										rel="noreferrer external"
									>
										{t("talkToSales", "Talk to sales")}
									</a>
								)}
							</div>
							<TrustRow />
						</div>
					)}
				</div>
			)}
		</div>
	);
}

/**
 * Globally mounted conversion dialog. Opened via `openUpgradeDialog()` /
 * `handleUpgradeRequiredError()` whenever a user hits a plan limit.
 *
 * Behaviour is steered by the hub's `conversion` config (flow-like.config.json):
 * consumer mode renders self-service upgrade tiers with Stripe checkout,
 * enterprise mode renders a contact card instead. Falls back to enterprise
 * mode when the hub has premium/self-service billing disabled.
 */
export function GlobalUpgradeDialog() {
	const isOpen = useUpgradeDialogStore((state) => state.isOpen);
	const trigger = useUpgradeDialogStore((state) => state.trigger);
	const close = useUpgradeDialogStore((state) => state.close);
	const setEnabled = useUpgradeDialogStore((state) => state.setEnabled);

	useEffect(() => {
		const showPlanLimit = (event: Event) => {
			handleUpgradeRequiredError((event as CustomEvent).detail);
		};
		window.addEventListener(PLAN_LIMIT_EVENT, showPlanLimit);
		return () => window.removeEventListener(PLAN_LIMIT_EVENT, showPlanLimit);
	}, []);
	const backend = useBackend();
	const auth = useAuth();
	const hub = useHub();
	const features = useFeatures();
	const isAuthenticated = auth?.isAuthenticated ?? false;

	const pricing = useInvoke(
		backend.userState.getPricing,
		backend.userState,
		[],
		isOpen && isAuthenticated,
	);

	const quota = useInvoke(
		backend.userState.getQuotaUsage,
		backend.userState,
		[],
		isAuthenticated &&
			(features.data?.premium ?? hub.hub?.features?.premium ?? false),
		[auth.user?.profile.sub],
	);
	useEffect(() => {
		if (
			!isAuthenticated ||
			!(features.data?.premium ?? hub.hub?.features?.premium ?? false)
		)
			return;
		const timer = setInterval(() => {
			if (document.visibilityState === "visible") quota.refetch();
		}, 60_000);
		return () => clearInterval(timer);
	}, [
		isAuthenticated,
		features.data?.premium,
		hub.hub?.features?.premium,
		quota.refetch,
	]);
	const hubConversion = hub.hub?.conversion;
	const conversion: IConversionInfo | undefined = pricing.data?.conversion;

	// The authenticated pricing response is authoritative; the hub-root doc is
	// the pre-open signal (pricing is only fetched while the dialog is open).
	const configEnabled = conversion?.enabled ?? hubConversion?.enabled;
	useEffect(() => {
		if (typeof configEnabled === "boolean") {
			setEnabled(configEnabled);
		}
	}, [configEnabled, setEnabled]);

	// Self-correct when the open raced ahead of config: the operator disabled
	// the conversion flow, so honor the "plain message" contract instead.
	useEffect(() => {
		if (isOpen && configEnabled === false) {
			close();
			if (trigger?.message) {
				toast.error(trigger.message);
			}
		}
	}, [isOpen, configEnabled, close, trigger?.message]);
	const premiumEnabled =
		features.data?.premium ?? hub.hub?.features?.premium ?? false;
	const mode: "consumer" | "enterprise" =
		(conversion?.mode ?? hubConversion?.mode) === "enterprise" ||
		!premiumEnabled
			? "enterprise"
			: "consumer";

	const contact: EnterpriseContact = {
		name:
			conversion?.contact_name ??
			hubConversion?.contact?.name ??
			hub.hub?.contact?.name,
		email:
			conversion?.contact_email ??
			hubConversion?.contact?.email ??
			hub.hub?.contact?.email,
		url:
			conversion?.contact_url ??
			hubConversion?.contact?.url ??
			hub.hub?.contact?.url,
		message:
			conversion?.contact_message ??
			hubConversion?.contact_message ??
			undefined,
	};

	const reason: UpgradeReason = trigger?.reason ?? "generic";
	const copy = REASON_COPY[reason] ?? REASON_COPY.generic;
	const headline =
		reason === "generic"
			? (conversion?.headline ?? hubConversion?.headline ?? copy.title)
			: copy.title;
	const subheadline =
		reason === "generic"
			? (conversion?.subheadline ?? hubConversion?.subheadline ?? copy.sub)
			: copy.sub;

	const currentTier = (pricing.data?.current_tier ?? "FREE").toUpperCase();
	const anotherPayer = quotaBelongsToAnotherPayer(
		trigger?.quota,
		auth.user?.profile.sub,
	);

	const requiredModelTier = trigger?.quota?.requiredModelTier;
	const upgradeTiers = useMemo(() => {
		const tiers = pricing.data?.tiers;
		if (!tiers) return [] as [string, ITierInfo][];
		const currentIndex = TIER_ORDER.indexOf(currentTier);
		return Object.entries(tiers)
			.filter(([key, tier]) => {
				if (!supportsHostedModel(tier, requiredModelTier)) return false;
				if (key === ENTERPRISE_TIER) return false;
				const index = TIER_ORDER.indexOf(key);
				return index === -1 || currentIndex === -1 || index > currentIndex;
			})
			.sort(([a], [b]) => TIER_ORDER.indexOf(a) - TIER_ORDER.indexOf(b));
	}, [pricing.data?.tiers, currentTier, requiredModelTier]);

	const emphasizedTier = useMemo(() => {
		if (requiredModelTier) return upgradeTiers[0]?.[0];
		const required = trigger?.requiredTier?.toUpperCase();
		if (required && upgradeTiers.some(([key]) => key === required)) {
			return required;
		}
		const highlighted = upgradeTiers.find(([, tier]) => tier.highlight);
		return highlighted?.[0] ?? upgradeTiers[0]?.[0];
	}, [trigger?.requiredTier, upgradeTiers, requiredModelTier]);

	const enterpriseTier = pricing.data?.tiers?.[ENTERPRISE_TIER];

	const [loadingTier, setLoadingTier] = useState<string | null>(null);

	// Checkout completes in the SYSTEM browser on desktop, so Stripe's
	// success/cancel redirect must target the hosted web app, never the
	// tauri:// origin of the webview.
	const checkoutReturnOrigin = useCallback(() => {
		if (isTauri()) {
			const appDomain = hub.hub?.app;
			if (appDomain) {
				return appDomain.startsWith("http")
					? appDomain.replace(/\/+$/, "")
					: `https://${appDomain}`;
			}
		}
		return window.location.origin;
	}, [hub.hub?.app]);

	const handleUpgrade = useCallback(
		async (tier: string, priceId?: string, interval?: "month" | "year") => {
			if (anotherPayer) return;
			setLoadingTier(tier);
			try {
				const origin = checkoutReturnOrigin();
				const response = await backend.userState.createSubscription({
					tier,
					price_id: priceId,
					interval,
					success_url: `${origin}/subscription?success=true`,
					cancel_url: `${origin}/subscription?canceled=true`,
				});
				await openExternalUrl(response.checkout_url, "checkout");
			} catch (error) {
				console.error("Failed to create subscription checkout:", error);
				toast.error("Failed to start checkout. Please try again.");
			} finally {
				setLoadingTier(null);
			}
		},
		[backend.userState, checkoutReturnOrigin, anotherPayer],
	);

	const handleManageBilling = useCallback(async () => {
		if (anotherPayer) return;
		try {
			const billingSession = await backend.userState.getBillingSession();
			await openExternalUrl(billingSession.url, "the billing portal");
		} catch (error) {
			console.error("Failed to get billing session:", error);
			toast.error("Failed to open the billing portal.");
		}
	}, [backend.userState, anotherPayer]);

	return (
		<Dialog open={isOpen} onOpenChange={(open) => !open && close()}>
			<QuotaWarnings overview={quota.data} />
			<DialogContent
				className={
					mode === "consumer" && !anotherPayer
						? "gap-0 p-0 sm:max-w-2xl max-h-[90dvh] overflow-y-auto"
						: "gap-0 p-0 sm:max-w-lg"
				}
			>
				{anotherPayer && trigger?.quota ? (
					<SharedPlanLimit quota={trigger.quota} onClose={close} />
				) : (
					<UpgradeDialogBody
						mode={mode}
						contact={contact}
						headline={headline}
						subheadline={subheadline}
						enterpriseHeadline={
							conversion?.headline ?? hubConversion?.headline ?? undefined
						}
						triggerMessage={trigger?.message}
						reason={reason}
						quota={trigger?.quota}
						hubName={hub.hub?.name}
						isAuthenticated={isAuthenticated}
						pricingStatus={
							pricing.isLoading
								? "loading"
								: pricing.isError
									? "error"
									: "ready"
						}
						onRetryPricing={() => pricing.refetch()}
						upgradeTiers={upgradeTiers}
						currentTier={currentTier}
						emphasizedTier={emphasizedTier}
						requiredModelTier={requiredModelTier}
						enterpriseTier={enterpriseTier}
						loadingTier={loadingTier}
						onUpgrade={handleUpgrade}
						onManageBilling={handleManageBilling}
						onClose={close}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}
