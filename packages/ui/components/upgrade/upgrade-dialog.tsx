"use client";

import {
	ArrowRight,
	Building2,
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
import { openExternalUrl } from "../../lib/open-external";
import { isTauri } from "../../lib/platform";
import { useBackend } from "../../state/backend-state";
import type {
	IConversionInfo,
	ITierInfo,
} from "../../state/backend-state/user-state";
import {
	type UpgradeReason,
	useUpgradeDialogStore,
} from "../../state/upgrade-dialog-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "../ui/dialog";
import { ENTERPRISE_TIER, TIER_ORDER, TierCard } from "./tier-card";

const REASON_COPY: Record<UpgradeReason, { title: string; sub: string }> = {
	"project-limit": {
		title: "You've outgrown your plan",
		sub: "Get room for more online projects — your existing work stays exactly where it is.",
	},
	"model-tier": {
		title: "Unlock more powerful AI models",
		sub: "This model is part of a higher plan. Upgrade to use it in your flows and chats.",
	},
	storage: {
		title: "You've reached your storage limit",
		sub: "Upgrade for more cloud storage and keep your projects in sync.",
	},
	executions: {
		title: "You've used all your cloud runs",
		sub: "Upgrade for more monthly cloud executions and faster machines.",
	},
	generic: {
		title: "Unlock the full power of your workspace",
		sub: "More projects, premium AI models and faster cloud executions.",
	},
};

export function TrustRow() {
	return (
		<div className="flex flex-wrap items-center justify-center gap-x-6 gap-y-2 text-xs text-muted-foreground">
			<span className="flex items-center gap-1.5">
				<ShieldCheck className="h-3.5 w-3.5" />
				Secure checkout via Stripe
			</span>
			<span className="flex items-center gap-1.5">
				<Zap className="h-3.5 w-3.5" />
				Instant activation
			</span>
			<span className="flex items-center gap-1.5">
				<Sparkles className="h-3.5 w-3.5" />
				Cancel anytime
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
	const message =
		contact.message ??
		`${hubName ?? "This workspace"} is managed by your organization. Reach out to ${
			contact.name ?? "your administrator"
		} to unlock more capacity for your account.`;
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
							{contactLabel ?? `Contact ${contact.name ?? "us"}`}
						</a>
					</Button>
				)}
				{contact.url && (
					<Button variant="outline" asChild>
						<a href={contact.url} target="_blank" rel="noreferrer external">
							Learn more
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
	enterpriseTier?: ITierInfo;
	loadingTier?: string | null;
	onUpgrade: (tier: string) => Promise<void> | void;
	onManageBilling: () => Promise<void> | void;
	onClose?: () => void;
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
	enterpriseTier,
	loadingTier,
	onUpgrade,
	onManageBilling,
	onClose,
}: Readonly<UpgradeDialogBodyProps>) {
	const gridColumns =
		upgradeTiers.length >= 3
			? "sm:grid-cols-3"
			: upgradeTiers.length === 2
				? "sm:grid-cols-2"
				: "sm:grid-cols-1";

	return (
		<div className="relative z-10 pt-8">
			{mode === "enterprise" ? (
				<EnterpriseContent
					contact={contact}
					headline={enterpriseHeadline}
					triggerMessage={triggerMessage}
					hubName={hubName}
					contactLabel="Contact Your Admin"
				/>
			) : (
				<div className="flex flex-col gap-6 px-8 pb-8">
					<div className="space-y-2 text-center">
						<DialogTitle className="text-2xl font-bold tracking-tight">
							{headline}
						</DialogTitle>
						<DialogDescription className="mx-auto max-w-lg text-sm text-muted-foreground">
							{subheadline}
						</DialogDescription>
					</div>

					{triggerMessage && (
						<div className="mx-auto w-full max-w-lg">
							<LimitCallout message={triggerMessage} />
						</div>
					)}

					{!isAuthenticated ? (
						<p className="py-4 text-center text-sm text-muted-foreground">
							Sign in to see upgrade options for your account.
						</p>
					) : pricingStatus === "loading" ? (
						<div className="flex items-center justify-center py-12">
							<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
						</div>
					) : pricingStatus === "error" ? (
						<div className="flex flex-col items-center gap-3 py-8 text-center">
							<p className="text-sm text-muted-foreground">
								Couldn't load upgrade options. Please check your connection.
							</p>
							<Button variant="outline" size="sm" onClick={onRetryPricing}>
								Try again
							</Button>
						</div>
					) : upgradeTiers.length > 0 ? (
						<div className={`grid gap-4 pt-2 ${gridColumns}`}>
							{upgradeTiers.map(([key, tier]) => (
								<TierCard
									key={key}
									tierKey={key}
									tier={tier}
									currentTier={currentTier}
									onUpgrade={onUpgrade}
									onManageBilling={onManageBilling}
									isLoading={loadingTier === key}
									compact
									emphasize={key === emphasizedTier}
								/>
							))}
						</div>
					) : (
						<EnterpriseContent
							contact={{
								...contact,
								message: `You're already on the highest self-serve plan. For enterprise capacity, governance and custom agreements, talk to ${contact.name ?? "sales"}.`,
							}}
							headline="You're on our highest plan"
							triggerMessage={undefined}
							hubName={hubName}
							asDialogTitle={false}
						/>
					)}

					{upgradeTiers.length > 0 && (
						<div className="flex flex-col items-center gap-4">
							{enterpriseTier && (
								<p className="text-xs text-muted-foreground">
									Need enterprise scale, governance or custom agreements?{" "}
									<a
										className="font-medium text-primary hover:underline"
										href={
											enterpriseTier.contact_url ??
											(contact.email ? `mailto:${contact.email}` : "#")
										}
										target="_blank"
										rel="noreferrer external"
									>
										Talk to sales
									</a>
								</p>
							)}
							<TrustRow />
							<Link
								href="/subscription"
								onClick={onClose}
								className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
							>
								Compare all plans
							</Link>
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

	const upgradeTiers = useMemo(() => {
		const tiers = pricing.data?.tiers;
		if (!tiers) return [] as [string, ITierInfo][];
		const currentIndex = TIER_ORDER.indexOf(currentTier);
		return Object.entries(tiers)
			.filter(([key]) => {
				if (key === ENTERPRISE_TIER) return false;
				const index = TIER_ORDER.indexOf(key);
				return index === -1 || currentIndex === -1 || index > currentIndex;
			})
			.sort(([a], [b]) => TIER_ORDER.indexOf(a) - TIER_ORDER.indexOf(b));
	}, [pricing.data?.tiers, currentTier]);

	const emphasizedTier = useMemo(() => {
		const required = trigger?.requiredTier?.toUpperCase();
		if (required && upgradeTiers.some(([key]) => key === required)) {
			return required;
		}
		const highlighted = upgradeTiers.find(([, tier]) => tier.highlight);
		return highlighted?.[0] ?? upgradeTiers[0]?.[0];
	}, [trigger?.requiredTier, upgradeTiers]);

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
		async (tier: string) => {
			setLoadingTier(tier);
			try {
				const origin = checkoutReturnOrigin();
				const response = await backend.userState.createSubscription({
					tier,
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
		[backend.userState, checkoutReturnOrigin],
	);

	const handleManageBilling = useCallback(async () => {
		try {
			const billingSession = await backend.userState.getBillingSession();
			await openExternalUrl(billingSession.url, "the billing portal");
		} catch (error) {
			console.error("Failed to get billing session:", error);
			toast.error("Failed to open the billing portal.");
		}
	}, [backend.userState]);

	return (
		<Dialog open={isOpen} onOpenChange={(open) => !open && close()}>
			<DialogContent
				className={
					mode === "consumer"
						? "gap-0 p-0 sm:max-w-3xl"
						: "gap-0 p-0 sm:max-w-lg"
				}
			>
				<UpgradeDialogBody
					mode={mode}
					contact={contact}
					headline={headline}
					subheadline={subheadline}
					enterpriseHeadline={
						conversion?.headline ?? hubConversion?.headline ?? undefined
					}
					triggerMessage={trigger?.message}
					hubName={hub.hub?.name}
					isAuthenticated={isAuthenticated}
					pricingStatus={
						pricing.isLoading ? "loading" : pricing.isError ? "error" : "ready"
					}
					onRetryPricing={() => pricing.refetch()}
					upgradeTiers={upgradeTiers}
					currentTier={currentTier}
					emphasizedTier={emphasizedTier}
					enterpriseTier={enterpriseTier}
					loadingTier={loadingTier}
					onUpgrade={handleUpgrade}
					onManageBilling={handleManageBilling}
					onClose={close}
				/>
			</DialogContent>
		</Dialog>
	);
}
