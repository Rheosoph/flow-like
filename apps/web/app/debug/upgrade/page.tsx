"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Dialog,
	DialogContent,
	UpgradeDialogBody,
} from "@flow-like/flow-like-ui";
import type { ITierInfo } from "@flow-like/flow-like-ui/state/backend-state/user-state";
import { useSearchParams } from "next/navigation";
import { Suspense, useMemo } from "react";

const SYNTHETIC_TIERS: Record<string, ITierInfo> = {
	PREMIUM: {
		name: "PREMIUM",
		display_name: "Premium",
		tagline: "For builders who ship every week",
		highlight: true,
		badge: "Most popular",
		features: [
			"200 online projects",
			"500 cloud runs per month",
			"1 GB cloud storage",
			"Premium AI models included",
			"Faster cloud executions",
		],
		product_id: "prod_debug_premium",
		max_non_visible_projects: 200,
		max_remote_executions: 500,
		execution_tier: "small",
		max_total_size: 1_000_000_000,
		max_llm_cost: 300,
		max_llm_calls: 1000,
		llm_tiers: ["PREMIUM", "FREE"],
		price: { amount: 999, currency: "eur", interval: "month" },
	},
	PRO: {
		name: "PRO",
		display_name: "Pro",
		tagline: "Scale your automations without limits",
		features: [
			"Unlimited online projects",
			"5,000 cloud runs per month",
			"10 GB cloud storage",
			"Pro-grade AI models included",
			"High-performance executions",
			"Priority support",
		],
		product_id: "prod_debug_pro",
		max_non_visible_projects: -1,
		max_remote_executions: 5000,
		execution_tier: "medium",
		max_total_size: 10_000_000_000,
		max_llm_cost: 3000,
		max_llm_calls: 10000,
		llm_tiers: ["PRO", "PREMIUM", "FREE"],
		price: { amount: 2999, currency: "eur", interval: "month" },
	},
	ENTERPRISE: {
		name: "ENTERPRISE",
		display_name: "Enterprise",
		tagline: "Security, control and scale for your organization",
		features: [
			"Unlimited projects, runs and storage",
			"Maximum execution performance",
			"All AI models included",
			"Governance & AI Act tooling",
			"Dedicated support & SLAs",
		],
		max_non_visible_projects: -1,
		max_remote_executions: -1,
		execution_tier: "max",
		max_total_size: -1,
		max_llm_cost: -1,
		llm_tiers: ["PRO", "PREMIUM", "ENTERPRISE", "FREE"],
		contact_url: "mailto:enterprise@flow-like.com",
	},
};

const TRIGGER_MESSAGES: Record<string, string> = {
	"project-limit":
		"You have used 10 of 10 online projects included in your plan.",
	"model-tier":
		"This model requires the PREMIUM tier, which is not included in your plan.",
};

/**
 * Capture-only route: renders the real upgrade dialog with synthetic pricing
 * so the doc-screenshot CLI can photograph it without auth or a backend.
 * Query params: mode=consumer|enterprise, reason=project-limit|model-tier|generic.
 */
function UpgradeDebugContent() {
	const { t } = useTranslation("common");
	const searchParams = useSearchParams();
	const mode =
		searchParams.get("mode") === "enterprise" ? "enterprise" : "consumer";
	const reason = searchParams.get("reason") ?? "project-limit";
	const triggerMessage = TRIGGER_MESSAGES[reason];

	const headline = useMemo(() => {
		if (reason === "model-tier") return t('unlockMorePowerfulAiModels', 'Unlock more powerful AI models');
		if (reason === "generic") return t('unlockTheFullPowerOfFlowlike', 'Unlock the full power of Flow-Like');
		return t('youveOutgrownYourPlan', 'You\'ve outgrown your plan');
	}, [reason]);

	const subheadline = useMemo(() => {
		if (reason === "model-tier")
			return t('thisModelIsPartOfAHigherPlanUpgradeToUseItInYourFlowsAndChats', 'This model is part of a higher plan. Upgrade to use it in your flows and chats.');
		if (reason === "generic")
			return t('moreProjectsPremiumAiModelsAndFasterCloudExecutionsUpgradeInSecondsCancelAnytime', 'More projects, premium AI models and faster cloud executions — upgrade in seconds, cancel anytime.');
		return t('getRoomForMoreOnlineProjectsYourExistingWorkStaysExactlyWhereItIs', 'Get room for more online projects — your existing work stays exactly where it is.');
	}, [reason]);

	return (
		<main className="h-screen w-screen bg-background">
			<Dialog open>
				<DialogContent
					className={
						mode === "consumer"
							? "gap-0 p-0 sm:max-w-3xl"
							: "gap-0 p-0 sm:max-w-lg"
					}
				>
					<UpgradeDialogBody
						mode={mode}
						contact={{
							name: t('enterpriseSales', 'Enterprise Sales'),
							email: "enterprise@flow-like.com",
							url: "https://flow-like.com",
							message:
								mode === "enterprise"
									? t('yourWorkspaceIsManagedByYourOrganizationReachOutToYourAdministratorToUnlockMoreProjectsModelsOrCapacity', 'Your workspace is managed by your organization. Reach out to your administrator to unlock more projects, models or capacity.')
									: undefined,
						}}
						headline={headline}
						subheadline={subheadline}
						triggerMessage={triggerMessage}
						hubName="Flow-Like"
						isAuthenticated
						pricingStatus="ready"
						upgradeTiers={[
							["PREMIUM", SYNTHETIC_TIERS.PREMIUM],
							["PRO", SYNTHETIC_TIERS.PRO],
						]}
						currentTier="FREE"
						emphasizedTier="PREMIUM"
						enterpriseTier={SYNTHETIC_TIERS.ENTERPRISE}
						onUpgrade={() => {}}
						onManageBilling={() => {}}
					/>
				</DialogContent>
			</Dialog>
		</main>
	);
}

export default function UpgradeDebugPage() {
	return (
		<Suspense fallback={null}>
			<UpgradeDebugContent />
		</Suspense>
	);
}
