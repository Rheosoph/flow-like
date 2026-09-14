import { afterAll, expect, mock, test } from "bun:test";
import type { HTMLAttributes } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { QuotaDetail } from "../../lib/quota";
import type { ITierInfo } from "../../state/backend-state/user-state";
import type { UpgradeDialogBodyProps } from "./upgrade-dialog";

mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_: string, value: string) => value }),
}));
mock.module("react-oidc-context", () => ({ useAuth: () => ({}) }));
mock.module("../../hooks/use-features", () => ({ useFeatures: () => ({}) }));
mock.module("../../hooks/use-hub", () => ({ useHub: () => ({}) }));
mock.module("../../hooks/use-invoke", () => ({ useInvoke: () => ({}) }));
mock.module("../../state/backend-state", () => ({ useBackend: () => ({}) }));
mock.module("./tier-card", () => ({
	ENTERPRISE_TIER: {},
	TIER_ORDER: [],
	TierCard: ({
		tierKey,
		focusResource,
	}: { tierKey: string; focusResource?: string }) => (
		<div data-tier={tierKey} data-focus={focusResource}>
			Choose {tierKey}
		</div>
	),
}));
mock.module("../ui/dialog", () => ({
	Dialog: () => null,
	DialogContent: () => null,
	DialogTitle: (props: HTMLAttributes<HTMLHeadingElement>) => <h2 {...props} />,
	DialogDescription: (props: HTMLAttributes<HTMLParagraphElement>) => (
		<p {...props} />
	),
}));
afterAll(() => mock.restore());

test("shared model access directs users to the owner without zero-limit counters or checkout", async () => {
	const { SharedPlanLimit } = await import("./upgrade-dialog");
	const quota: QuotaDetail = {
		resource: "hosted_model_access",
		scope: "account",
		payerId: "owner",
		plan: "FREE",
		used: 0,
		reserved: 0,
		requested: 1,
		limit: 0,
		unit: "access",
	};
	const markup = renderToStaticMarkup(
		<SharedPlanLimit quota={quota} onClose={() => {}} />,
	);
	expect(markup).toContain("This model is not included in the app");
	expect(markup).toContain("billing owner");
	expect(markup).toContain(
		"Changing your personal subscription will not increase",
	);
	expect(markup).toContain("Copy details for billing owner");
	expect(markup).toContain(
		"Choose an included model or connect your own provider",
	);
	expect(markup).not.toContain("0 used");
	expect(markup).not.toContain("limit 0");
	expect(markup).not.toContain("Renews");
	expect(markup).not.toContain("Checkout");
});

const paidTier = (
	name: string,
	budget: number,
	llm_tiers = ["FREE", "PREMIUM"],
): ITierInfo => ({
	name,
	max_ai_cost_micros: budget,
	max_llm_cost: budget / 10_000,
	llm_tiers,
	max_non_visible_projects: 200,
	max_remote_executions: 10_000,
	execution_tier: "small",
	max_total_size: 25_000_000_000,
});
const props: UpgradeDialogBodyProps & { quota: QuotaDetail } = {
	mode: "consumer",
	contact: { email: "sales@example.com" },
	headline: "Your hosted AI allowance is used up",
	subheadline: "Choose more hosted AI",
	currentTier: "FREE",
	isAuthenticated: true,
	pricingStatus: "ready",
	onUpgrade: () => {},
	onManageBilling: () => {},
	reason: "ai-budget",
	emphasizedTier: "PRO",
	upgradeTiers: [
		["PREMIUM", paidTier("Premium", 3_000_000)],
		["PRO", paidTier("Pro", 10_000_000, ["PRO", "PREMIUM", "FREE"])],
	],
	quota: {
		resource: "hosted_ai_cost_micros",
		scope: "account",
		payerId: "self",
		plan: "FREE",
		used: 1_000_000,
		reserved: 0,
		limit: 1_000_000,
		unit: "microEUR",
		periodEnd: "2026-10-01T00:00:00Z",
	},
};

test("quota upgrade recommends the first sufficient plan and keeps alternatives and free tips collapsed", async () => {
	const { UpgradeDialogBody } = await import("./upgrade-dialog");
	const markup = renderToStaticMarkup(<UpgradeDialogBody {...props} />);
	expect(markup).toMatch(/recommended-upgrade[\s\S]*?data-tier="PREMIUM"/);
	expect(markup).toContain("→");
	expect(markup).toContain("per month");
	expect(markup).toContain('data-focus="hosted_ai_cost_micros"');
	expect(markup).toContain("See other plans");
	expect(markup).not.toMatch(/<details[^>]* open/);
	expect(markup.indexOf('data-tier="PREMIUM"')).toBeLessThan(
		markup.indexOf("More ways to keep building"),
	);
	expect(markup).toContain("Your own models use no Flow-Like AI allowance");
	expect(markup).not.toContain("T00:00");
});

test("upgrade recommendation covers pending usage and required model access", async () => {
	const { UpgradeDialogBody } = await import("./upgrade-dialog");
	const larger = renderToStaticMarkup(
		<UpgradeDialogBody
			{...props}
			quota={{ ...props.quota, reserved: 2_500_000, requested: 100_000 }}
		/>,
	);
	expect(larger).not.toContain('data-tier="PREMIUM"');
	expect(larger).toContain('data-tier="PRO"');
	const model = renderToStaticMarkup(
		<UpgradeDialogBody
			{...props}
			requiredModelTier="PRO"
			reason="model-tier"
			quota={{
				...props.quota,
				resource: "hosted_model_access",
				used: 0,
				limit: 0,
				periodEnd: undefined,
			}}
		/>,
	);
	expect(model).not.toContain('data-tier="PREMIUM"');
	expect(model).toContain('data-tier="PRO"');
	expect(model).toContain("Includes access to this hosted model");
	expect(model).not.toContain("0 of 0");
	expect(model).not.toContain("Renews");
});

test("unavailable capacity and pricing errors do not show a checkout recommendation", async () => {
	const { UpgradeDialogBody } = await import("./upgrade-dialog");
	const capacity = renderToStaticMarkup(
		<UpgradeDialogBody
			{...props}
			quota={{ ...props.quota, requested: 20_000_000 }}
		/>,
	);
	expect(capacity).not.toContain("recommended-upgrade");
	expect(capacity).toContain("mailto:sales@example.com");
	const unknownCapacity = renderToStaticMarkup(
		<UpgradeDialogBody
			{...props}
			quota={{ ...props.quota, resource: "cloud_runtime_ms" }}
		/>,
	);
	expect(unknownCapacity).not.toContain("recommended-upgrade");
	const error = renderToStaticMarkup(
		<UpgradeDialogBody {...props} pricingStatus="error" />,
	);
	expect(error).not.toContain("data-tier=");
	expect(error).toContain("Try again");
	expect(error).toContain("More ways to keep building");
});
