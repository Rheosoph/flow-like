import { afterAll, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import type { HTMLAttributes } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { QuotaDetail } from "../../lib/quota";
import type { ITierInfo } from "../../state/backend-state/user-state";
import type { UpgradeDialogTrigger } from "../../state/upgrade-dialog-state";
import type { UpgradeDialogBodyProps } from "./upgrade-dialog";

// bun keeps a module mock for every later file in the process, so each mocked module is
// captured first and put back in afterAll. Radix picks its layout effect when first imported,
// so the real modules load under a document.
const documentDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"document",
);
Object.assign(globalThis, { document: new Window().document });
const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	oidc: { ...(await import("react-oidc-context")) },
	features: { ...(await import("../../hooks/use-features")) },
	hub: { ...(await import("../../hooks/use-hub")) },
	invoke: { ...(await import("../../hooks/use-invoke")) },
	backendState: { ...(await import("../../state/backend-state")) },
	tierCard: { ...(await import("./tier-card")) },
	dialog: { ...(await import("../ui/dialog")) },
	upgradeStore: { ...(await import("../../state/upgrade-dialog-state")) },
};
let globalTrigger: UpgradeDialogTrigger | undefined;
let pricingEnabled: boolean | undefined;
if (documentDescriptor)
	Object.defineProperty(globalThis, "document", documentDescriptor);
else Reflect.deleteProperty(globalThis, "document");

mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: (_: string, value: string) => value }),
}));
mock.module("react-oidc-context", () => ({
	...actual.oidc,
	useAuth: () => ({
		isAuthenticated: true,
		user: { profile: { sub: "self" } },
	}),
}));
mock.module("../../hooks/use-features", () => ({
	...actual.features,
	useFeatures: () => ({}),
}));
mock.module("../../hooks/use-hub", () => ({
	...actual.hub,
	useHub: () => ({
		hub: {
			features: { premium: true },
			conversion: { enabled: false },
			contact: { email: "admin@example.com" },
		},
	}),
}));
mock.module("../../hooks/use-invoke", () => ({
	...actual.invoke,
	useInvoke: (
		fn: (...args: unknown[]) => unknown,
		_context: unknown,
		_args: unknown[],
		enabled: boolean,
	) => {
		if (fn.name === "getPricing") pricingEnabled = enabled;
		return {};
	},
}));
mock.module("../../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => ({
		userState: { async getPricing() {}, async getQuotaUsage() {} },
	}),
}));
mock.module("../../state/upgrade-dialog-state", () => ({
	...actual.upgradeStore,
	useUpgradeDialogStore: (selector: (state: unknown) => unknown) =>
		selector({
			isOpen: true,
			trigger: globalTrigger,
			close() {},
			setEnabled() {},
		}),
}));
mock.module("./tier-card", () => ({
	...actual.tierCard,
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
	...actual.dialog,
	Dialog: ({ children }: HTMLAttributes<HTMLDivElement>) => (
		<div>{children}</div>
	),
	DialogContent: ({ children }: HTMLAttributes<HTMLDivElement>) => (
		<div>{children}</div>
	),
	DialogTitle: (props: HTMLAttributes<HTMLHeadingElement>) => <h2 {...props} />,
	DialogDescription: (props: HTMLAttributes<HTMLParagraphElement>) => (
		<p {...props} />
	),
}));
afterAll(() => {
	mock.restore();
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("react-oidc-context", () => actual.oidc);
	mock.module("../../hooks/use-features", () => actual.features);
	mock.module("../../hooks/use-hub", () => actual.hub);
	mock.module("../../hooks/use-invoke", () => actual.invoke);
	mock.module("../../state/backend-state", () => actual.backendState);
	mock.module("./tier-card", () => actual.tierCard);
	mock.module("../ui/dialog", () => actual.dialog);
	mock.module("../../state/upgrade-dialog-state", () => actual.upgradeStore);
});

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

test("scoped monthly budgets show usage and administrator contact without paid plans", async () => {
	const { UpgradeDialogBody } = await import("./upgrade-dialog");
	for (const scope of ["app", "bot", "bot_group"]) {
		for (const mode of ["consumer", "enterprise"] as const) {
			const markup = renderToStaticMarkup(
				<UpgradeDialogBody
					{...props}
					mode={mode}
					contact={{
						name: "Enterprise support",
						email: "admin@example.com",
						url: "https://example.com/admin-help",
					}}
					quota={{
						...props.quota,
						scope,
						payerId: "another-owner",
						plan: "enterprise_policy",
						reserved: 250_000,
						requested: 10_000,
						actions: ["contact_admin"],
					}}
				/>,
			);
			expect(markup).toContain("monthly budget");
			expect(markup).toContain("Reserved");
			expect(markup).toContain("Monthly limit");
			expect(markup).toContain("This request");
			expect(markup).toContain("Resets");
			expect(markup).toContain("UTC");
			expect(markup).toContain("mailto:admin@example.com");
			expect(markup).toContain("https://example.com/admin-help");
			expect(markup).toContain("Copy budget details");
			expect(markup).not.toContain("data-tier=");
			expect(markup).not.toContain("recommended-upgrade");
			expect(markup).not.toContain("billing owner");
			expect(markup).not.toContain("enterprise_policy");
			expect(markup).not.toContain("More ways to keep building");
		}
	}
});

test("a monthly budget prompt needs no pricing response or configured contact", async () => {
	const { UpgradeDialogBody } = await import("./upgrade-dialog");
	const markup = renderToStaticMarkup(
		<UpgradeDialogBody
			{...props}
			contact={{}}
			pricingStatus="error"
			quota={{ ...props.quota, scope: "bot" }}
		/>,
	);
	expect(markup).toContain("Contact your enterprise administrator");
	expect(markup).toContain("Copy budget details");
	expect(markup).not.toContain("Couldn't load upgrade options");
	expect(markup).not.toContain("data-tier=");
});

test("the global dialog gives scoped budgets precedence over another payer and skips pricing", async () => {
	const { GlobalUpgradeDialog } = await import("./upgrade-dialog");
	const client = new QueryClient();
	try {
		for (const scope of ["app", "bot", "bot_group"]) {
			globalTrigger = {
				reason: "ai-budget",
				quota: {
					...props.quota,
					payerId: "another-owner",
					scope,
					plan: "enterprise_policy",
				},
			};
			const markup = renderToStaticMarkup(
				<QueryClientProvider client={client}>
					<GlobalUpgradeDialog />
				</QueryClientProvider>,
			);
			expect(markup).toContain("monthly budget");
			expect(markup).toContain("mailto:admin@example.com");
			expect(markup).not.toContain("billing owner");
			expect(markup).not.toContain("data-tier=");
			expect(pricingEnabled).toBe(false);
		}
		globalTrigger = {
			reason: "ai-budget",
			quota: { ...props.quota, payerId: "another-owner", scope: "account" },
		};
		const account = renderToStaticMarkup(
			<QueryClientProvider client={client}>
				<GlobalUpgradeDialog />
			</QueryClientProvider>,
		);
		expect(account).toContain("billing owner");
		expect(account).not.toContain("Copy budget details");
		expect(pricingEnabled).toBe(true);
	} finally {
		globalTrigger = undefined;
		client.clear();
	}
});
