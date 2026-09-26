import { afterAll, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { renderToStaticMarkup } from "react-dom/server";
import type {
	IFlowPaymentsReport,
	ISalesOverview,
} from "../../../state/backend-state/sales-state";

// bun keeps a module mock for every later file in the process, so the real module is
// captured first and put back in afterAll.
const actualLocales = { ...(await import("@flow-like/locales")) };
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
});

mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({
		t: (key: string, fallback: string, options?: Record<string, unknown>) =>
			(fallback ?? key).replace(/\{\{(\w+)\}\}/g, (match, name: string) =>
				String(options?.[name] ?? match),
			),
		i18n: { language: "en", resolvedLanguage: "en" },
	}),
}));

// Radix picks its layout effect when first imported, so the component loads under a document.
const documentDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"document",
);
Object.assign(globalThis, { document: new Window().document });
const { OverviewTab } = await import("./overview-tab");
if (documentDescriptor)
	Object.defineProperty(globalThis, "document", documentDescriptor);
else Reflect.deleteProperty(globalThis, "document");

const flows: IFlowPaymentsReport = {
	totalRevenue: 3000,
	totalCollected: 3500,
	totalRefunded: 500,
	totalPayments: 3,
	refundedPayments: 1,
	uniquePayers: 2,
	periodRevenue: 1000,
	periodPayments: 2,
	revenueChangePercent: -50,
	paymentsChangePercent: 100,
	dailyStats: [
		{
			date: "2026-09-20",
			revenue: 1000,
			collected: 1000,
			refunded: 0,
			payments: 1,
		},
	],
	recentPayments: [
		{
			id: "a1",
			productName: "Report export",
			reference: "INV-1",
			payerUserId: "user-1",
			payerName: "Ada",
			payerAvatar: null,
			boardId: "b1",
			runId: "run1",
			amount: 1000,
			collected: 1000,
			refunded: 0,
			currency: "eur",
			createdAt: Date.UTC(2026, 8, 20),
		},
		{
			id: "a2",
			productName: "Premium lookup",
			reference: null,
			payerUserId: "user-2",
			payerName: null,
			payerAvatar: null,
			boardId: "b1",
			runId: "run2",
			amount: 500,
			collected: 500,
			refunded: 500,
			currency: "eur",
			createdAt: Date.UTC(2026, 8, 19),
		},
	],
};

const overview: ISalesOverview = {
	totalRevenue: 4990,
	totalPurchases: 10,
	totalRefunds: 0,
	refundAmount: 0,
	netRevenue: 4990,
	uniqueBuyers: 9,
	avgOrderValue: 499,
	currentPrice: 499,
	totalDiscounts: 0,
	totalMembers: 3,
	periodRevenue: 998,
	periodPurchases: 2,
	revenueChangePercent: 10,
	purchasesChangePercent: 5,
};

const render = (props: Partial<Parameters<typeof OverviewTab>[0]>) =>
	renderToStaticMarkup(
		<OverviewTab
			store={null}
			flows={null}
			storeListed={false}
			dateRange="30d"
			onDateRangeChange={() => {}}
			{...props}
		/>,
	);

test("an unlisted app still shows what its flows collected", () => {
	const markup = render({ flows });

	expect(markup).toContain("Recent flow payments");
	expect(markup).toContain("Report export");
	expect(markup).toContain("INV-1");
	expect(markup).toContain("Ada");
	expect(markup).toContain("Refunded");
	expect(markup).toContain("€30.00");
	expect(markup).toContain("Store sales appear here once the app is public");
	expect(markup).not.toContain("Recent store purchases");
});

test("store and flow revenue add up, with each source named", () => {
	const markup = render({
		storeListed: true,
		flows,
		store: { overview, dailyStats: [], purchases: [], purchaseTotal: 10 },
	});

	expect(markup).toContain("€79.90");
	expect(markup).toContain("Store €49.90 · Flows €30.00");
	expect(markup).toContain("Recent store purchases");
	expect(markup).toContain("Recent flow payments");
	expect(markup).not.toContain("Store sales appear here");
});

test("a listed app whose revenue failed to load says so instead of inviting a listing", () => {
	const markup = render({ storeListed: true });

	expect(markup).toContain("Revenue could not be loaded");
	expect(markup).not.toContain("Make the app public");
});
