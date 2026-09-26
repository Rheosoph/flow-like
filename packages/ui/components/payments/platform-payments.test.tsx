import { afterAll, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

let responses: Record<string, unknown>;

// bun keeps a module mock for every later file in the process, so each mocked module is
// captured first and put back in afterAll.
const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	payments: { ...(await import("./use-payments")) },
	nextNavigation: { ...(await import("next/navigation")) },
	nextLink: { ...(await import("next/link")) },
	appPermissions: { ...(await import("../../hooks/use-app-permissions")) },
};
afterAll(() => {
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("./use-payments", () => actual.payments);
	mock.module("next/navigation", () => actual.nextNavigation);
	mock.module("next/link", () => actual.nextLink);
	mock.module("../../hooks/use-app-permissions", () => actual.appPermissions);
});

mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({
		t: (
			key: string,
			fallback: string | { defaultValue: string },
			options?: Record<string, unknown>,
		) => {
			const text =
				typeof fallback === "string"
					? fallback
					: (fallback?.defaultValue ?? key);
			return text.replace(/\{\{(\w+)\}\}/g, (match, name: string) =>
				String(options?.[name] ?? match),
			);
		},
		i18n: { language: "en", resolvedLanguage: "en" },
	}),
}));
mock.module("./use-payments", () => ({
	...actual.payments,
	usePayments: () => ({
		identity: ["test", "owner"],
		config: { onboarding_enabled: true, marketplace_enabled: true },
	}),
	usePaymentQuery: (path: string) => ({ data: responses[path] }),
	usePaymentDistribution: () => true,
}));
mock.module("next/navigation", () => ({
	...actual.nextNavigation,
	useSearchParams: () => new URLSearchParams("id=app"),
}));
mock.module("next/link", () => ({
	...actual.nextLink,
	default: ({ children, href }: { children: ReactNode; href: string }) => (
		<a href={href}>{children}</a>
	),
}));
mock.module("../../hooks/use-app-permissions", () => ({
	...actual.appPermissions,
	useAppPermissions: () => ({ can: () => true }),
}));

// Radix picks its layout effect when first imported, so the pages load under a document.
const documentDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"document",
);
Object.assign(globalThis, { document: new Window().document });
const { PayoutsPage } = await import("./payouts-page");
const { AppPaymentSettingsPanel, SellerTermsCard, SellerTermsConsentCard } =
	await import("./app-payment-settings");
const { EarningsPage } = await import("./earnings-page");
const { NodePaymentCard } = await import("./node-payment");
const { WithdrawalConfirmation, PurchaseLookupResult, PurchasesPage } =
	await import("./purchases-page");
const { marketplaceCheckoutPath } = await import("./checkout-dialog");
if (documentDescriptor)
	Object.defineProperty(globalThis, "document", documentDescriptor);
else Reflect.deleteProperty(globalThis, "document");

beforeEach(() => {
	responses = {};
});

test("platform admins never receive connected-account onboarding or payout prompts", () => {
	responses["user/payments/connect"] = {
		platformOwned: true,
		state: "platform",
		canAcceptPayments: false,
		canSell: false,
		payoutsEnabled: false,
	};
	const markup = renderToStaticMarkup(<PayoutsPage />);
	expect(markup).toContain("Proceeds belong to Flow-Like");
	expect(markup).not.toContain("Set up payments with Stripe");
	expect(markup).not.toContain("Disconnect payments");
	expect(markup).not.toContain("Reconnect existing account");
	expect(markup).not.toContain("Bank payouts");
});

test("independent sellers retain connected-account onboarding", () => {
	responses["user/payments/connect"] = {
		platformOwned: false,
		state: "not_started",
		canAcceptPayments: false,
		canSell: false,
		payoutsEnabled: false,
	};
	const markup = renderToStaticMarkup(<PayoutsPage />);
	expect(markup).toContain("Set up payments with Stripe");
	expect(markup).not.toContain("Proceeds belong to Flow-Like");
});

test("platform-owned app settings omit seller self-agreements and personal account links", () => {
	responses["apps/app/payments/readiness"] = {
		platformOwned: true,
		canAcceptPayments: true,
		canSell: true,
	};
	const markup = renderToStaticMarkup(
		<>
			<AppPaymentSettingsPanel appId="app" />
			<SellerTermsCard appId="app" />
		</>,
	);
	expect(markup).toContain("Flow-Like can collect payments for this app");
	expect(markup).not.toContain("Accept seller terms");
	expect(markup).not.toContain("Manage your payment account");
});

test("independent owners get the seller terms and their payout account link", () => {
	responses["apps/app/payments/readiness"] = {
		platformOwned: false,
		canAcceptPayments: false,
		canSell: false,
	};
	const panel = renderToStaticMarkup(<AppPaymentSettingsPanel appId="app" />);
	expect(panel).toContain("Manage your payment account");
	expect(panel).toContain("The owner must complete payment setup");
	expect(renderToStaticMarkup(<SellerTermsCard appId="app" />)).toContain(
		"Accept seller terms",
	);
});

test("package seller terms follow the seller account, not an app readiness row", () => {
	const card = (platformOwned: boolean | undefined) =>
		renderToStaticMarkup(
			<SellerTermsConsentCard
				termsPath="registry/package/pkg/marketplace/terms"
				platformOwned={platformOwned}
				description="Accept the current seller terms before you sell this package."
			/>,
		);
	expect(card(false)).toContain("Accept seller terms");
	expect(card(false)).toContain("before you sell this package");
	expect(card(true)).toBe("");
	expect(card(undefined)).toBe("");
});

test("marketplace checkout targets the item's own checkout route", () => {
	expect(marketplaceCheckoutPath("APP", "app/1")).toBe(
		"apps/app%2F1/marketplace/checkout",
	);
	expect(marketplaceCheckoutPath("PACKAGE", "com.example.pkg")).toBe(
		"registry/package/com.example.pkg/marketplace/checkout",
	);
});

test("package orders link to the package store page and name the package", () => {
	responses["user/purchases/pkg-order"] = {
		orderId: "pkg-order",
		appId: "",
		itemKind: "PACKAGE",
		itemId: "com.example.pkg",
		status: "COMPLETED",
		amount: 500,
		currency: "eur",
		refundedAmount: 0,
		pendingRefundAmount: 0,
		withdrawable: false,
	};
	responses["user/purchases/app-order"] = {
		orderId: "app-order",
		appId: "app-1",
		itemKind: "APP",
		itemName: "Some app",
		status: "COMPLETED",
		amount: 500,
		currency: "eur",
		refundedAmount: 0,
		pendingRefundAmount: 0,
		withdrawable: false,
	};
	const pkg = renderToStaticMarkup(
		<PurchaseLookupResult orderId="pkg-order" />,
	);
	expect(pkg).toContain('href="/store/packages?id=com.example.pkg"');
	expect(pkg).toContain(">com.example.pkg</a>");
	const app = renderToStaticMarkup(
		<PurchaseLookupResult orderId="app-order" />,
	);
	expect(app).toContain('href="/store?id=app-1"');
});

test("platform balance and historical recipients remain visibly distinct", () => {
	responses["user/payments/balance"] = {
		platformOwned: true,
		available: [{ currency: "eur", amount: 200 }],
		pending: [],
	};
	responses["user/payments/earnings"] = {
		platformOwned: true,
		entries: [true, false].map((platformOwned, index) => ({
			id: String(index),
			platformOwned,
			sourceType: "REQUEST",
			status: "PAID",
			amount: 100,
			currency: "eur",
			applicationFeeAmount: 0,
			createdAt: 0,
		})),
	};
	const markup = renderToStaticMarkup(<EarningsPage />);
	expect(markup).toContain("Flow-Like platform balance");
	expect(markup).toContain("Recipient: Flow-Like");
	expect(markup).toContain("Recipient: your connected Stripe account");
	expect(markup).not.toContain("This balance belongs to your Stripe account");
});

test("a platform payment names Flow-Like as the payer's recipient", () => {
	responses["payments/request?appId=app&runId=run"] = {
		id: "request",
		appId: "app",
		runId: "run",
		platformOwned: true,
		payeeUserId: "admin-personal-id",
		payeeDisplayName: "Admin Personal Name",
		status: "CREATED",
		expiresAt: Date.now() + 60_000,
		amountMinor: 500,
		currency: "eur",
		productName: "Example",
	};
	const markup = renderToStaticMarkup(
		<NodePaymentCard
			reference={{ id: "request", appId: "app", runId: "run" }}
		/>,
	);
	expect(markup).toContain("pay the amount above to Flow-Like");
	expect(markup).not.toContain("Admin Personal Name");
	expect(markup).not.toContain("admin-personal-id");
});

test("withdrawal confirmation identifies the contract and collects a name and receipt channel without a reason", () => {
	const markup = renderToStaticMarkup(
		<WithdrawalConfirmation
			order={{
				orderId: "order-123",
				appId: "app",
				status: "COMPLETED",
				amount: 1000,
				currency: "eur",
				refundedAmount: 0,
				pendingRefundAmount: 0,
				withdrawable: true,
			}}
			busy={false}
			onConfirm={() => {}}
			onCancel={() => {}}
		/>,
	);
	expect(markup).toContain("Order order-123");
	expect(markup).toContain("I withdraw from the purchase identified above.");
	const nameInput =
		markup.match(/<input[^>]*name="consumerName"[^>]*>/)?.[0] ?? "";
	const emailInput =
		markup.match(/<input[^>]*name="confirmationEmail"[^>]*>/)?.[0] ?? "";
	expect(nameInput).toContain('required=""');
	expect(nameInput).toContain('maxLength="200"');
	expect(nameInput).toContain('pattern=".*\\S.*"');
	expect(emailInput).toContain('type="email"');
	expect(emailInput).toContain('required=""');
	expect(emailInput).toContain('maxLength="254"');
	expect(markup).not.toContain('name="reason"');
	expect(markup).not.toContain('type="checkbox"');
	expect(markup.indexOf("Order order-123")).toBeLessThan(
		markup.indexOf("Confirm withdrawal"),
	);
	expect(markup).toContain("receipt date and time");
});

test("a purchase outside the recent list remains reachable with its withdrawal action", () => {
	responses["user/purchases"] = { orders: [] };
	responses["user/purchases/older%2Forder%3Fref%3D1"] = {
		orderId: "older/order?ref=1",
		appId: "app",
		itemName: "Earlier app",
		status: "COMPLETED",
		amount: 1000,
		currency: "eur",
		refundedAmount: 0,
		pendingRefundAmount: 0,
		withdrawable: true,
	};
	const page = renderToStaticMarkup(<PurchasesPage />);
	expect(page).toContain("Find a purchase");
	expect(page).toContain('name="orderReference"');
	expect(page).toContain("confirmation email");
	const result = renderToStaticMarkup(
		<PurchaseLookupResult orderId="older/order?ref=1" />,
	);
	expect(result).toContain("Earlier app");
	expect(result).toContain("Order older/order?ref=1");
	expect(result).toContain("Withdraw from contract");
});
