"use client";

import { useTranslation } from "@flow-like/locales";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInvoke } from "../../../hooks/use-invoke";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { asArray, isRecord } from "../../../lib/response-shape";
import { IAppVisibility } from "../../../lib/schema/app/app";
import { useBackend } from "../../../state/backend-state";
import type {
	ICreateDiscountRequest,
	IDiscount,
	IFlowPaymentsReport,
} from "../../../state/backend-state/sales-state";
import {
	AppPaymentSettingsPanel,
	paymentsFeatureEnabled,
} from "../../payments/app-payment-settings";
import { usePayments } from "../../payments/use-payments";
import { Skeleton } from "../../ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import { SectionLockedPanel } from "../permission/permission-gate";
import { type DateRange, OverviewTab, type StoreSales } from "./overview-tab";
import { PricingTab } from "./pricing-tab";
import { DiscountDialog, PriceEditorDialog } from "./sales-dialogs";

type MonetizationTab = "overview" | "pricing" | "settings";

const RANGE_DAYS: Record<DateRange, number> = { "7d": 7, "30d": 30, "90d": 90 };

function rangeBounds(range: DateRange) {
	const day = (ms: number) => new Date(ms).toISOString().split("T")[0];
	const now = Date.now();
	return {
		startDate: day(now - RANGE_DAYS[range] * 24 * 60 * 60 * 1000),
		endDate: day(now),
	};
}

function readTab(value: string | null): MonetizationTab {
	return value === "pricing" || value === "settings" ? value : "overview";
}

function LoadingSkeleton() {
	return (
		<div className="space-y-6">
			<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
				<Skeleton className="h-32" />
				<Skeleton className="h-32" />
				<Skeleton className="h-32" />
				<Skeleton className="h-32" />
			</div>
			<Skeleton className="h-[300px]" />
		</div>
	);
}

/**
 * Everything that earns this app money, in one place: store sales and flow
 * payments side by side, the store price and discounts, and the payment
 * settings that decide what flows may charge.
 */
export function SalesDashboard() {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const salesState = backend.salesState;
	const searchParams = useSearchParams();
	const appId = searchParams.get("id");

	const permissions = useAppPermissions(appId);
	// Every sales route is guarded by `verify_revenue_access`, which is NOT a
	// permission bit: it admits the app's owner role and never calls
	// `has_permission`. The client cannot reproduce that (the caller's own role
	// carries no `app.owner_role_id`), so it mirrors the Owner check plus the
	// role-name branch — a role named "Owner" that somehow lacks the bit still
	// passes server-side and must not be locked out here. The residual gap runs
	// the other way: an Admin-bit role passes this gate and is still refused by
	// the server (overview.rs TODO).
	const canReadSales =
		permissions.can(RolePermissions.Owner) ||
		permissions.roleName?.toLowerCase() === "owner";

	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[appId ?? ""],
		!!appId,
	);
	const visibility = app.data?.visibility;
	const storeListed =
		visibility === IAppVisibility.Public ||
		visibility === IAppVisibility.PublicRequestAccess;
	const paymentsOn = paymentsFeatureEnabled(usePayments().config);

	const [tab, setTab] = useState<MonetizationTab>(() =>
		readTab(searchParams.get("tab")),
	);
	const activeTab = tab === "settings" && !paymentsOn ? "overview" : tab;

	const [loading, setLoading] = useState(true);
	const [store, setStore] = useState<StoreSales | null>(null);
	const [discounts, setDiscounts] = useState<IDiscount[]>([]);
	const [flows, setFlows] = useState<IFlowPaymentsReport | null>(null);
	const [dateRange, setDateRange] = useState<DateRange>("30d");

	const [discountDialogOpen, setDiscountDialogOpen] = useState(false);
	const [editingDiscount, setEditingDiscount] = useState<
		IDiscount | undefined
	>();
	const [priceDialogOpen, setPriceDialogOpen] = useState(false);

	const loadError = useCallback(
		(error: unknown) =>
			toast.error(
				error instanceof Error
					? error.message
					: t("failedToLoadSalesData", "Failed to load sales data"),
			),
		[t],
	);

	const loadData = useCallback(async () => {
		if (!appId || !salesState) return;
		// Wait for the role and the app row rather than spending a 403 or a
		// "public apps only" 400 on the first paint.
		if (permissions.isLoading || !visibility) return;
		if (!canReadSales) {
			setLoading(false);
			return;
		}

		setLoading(true);
		const { startDate, endDate } = rangeBounds(dateRange);
		const [storeResult, flowResult] = await Promise.allSettled([
			storeListed
				? Promise.all([
						salesState.getSalesOverview(appId),
						salesState.getSalesStats(appId, startDate, endDate),
						salesState.listPurchases(appId, undefined, 0, 50),
						salesState.listDiscounts(appId),
					])
				: Promise.resolve(null),
			paymentsOn
				? salesState.getFlowPayments(appId, startDate, endDate)
				: Promise.resolve(null),
		]);

		if (storeResult.status === "fulfilled") {
			const value = storeResult.value;
			setStore(
				value && isRecord(value[0])
					? {
							overview: value[0],
							dailyStats: asArray(value[1]?.dailyStats),
							purchases: asArray(value[2]?.purchases),
							purchaseTotal: value[2]?.total ?? 0,
						}
					: null,
			);
			setDiscounts(asArray(value?.[3]));
		} else {
			setStore(null);
			loadError(storeResult.reason);
		}

		if (flowResult.status === "fulfilled") {
			const report = flowResult.value;
			setFlows(
				isRecord(report)
					? {
							...report,
							dailyStats: asArray(report.dailyStats),
							recentPayments: asArray(report.recentPayments),
						}
					: null,
			);
		} else {
			setFlows(null);
			loadError(flowResult.reason);
		}
		setLoading(false);
	}, [
		appId,
		canReadSales,
		permissions.isLoading,
		visibility,
		storeListed,
		paymentsOn,
		dateRange,
		salesState,
		loadError,
	]);

	useEffect(() => {
		loadData();
	}, [loadData]);

	const changeTab = useCallback((value: string) => {
		const next = readTab(value);
		setTab(next);
		const url = new URL(window.location.href);
		if (next === "overview") url.searchParams.delete("tab");
		else url.searchParams.set("tab", next);
		window.history.replaceState(null, "", url);
	}, []);

	const handleCreateDiscount = useCallback(
		async (discount: ICreateDiscountRequest) => {
			if (!appId || !salesState) return;
			await salesState.createDiscount(appId, discount);
			toast.success(t("discountCreated", "Discount created"));
			loadData();
		},
		[appId, salesState, loadData, t],
	);

	const handleUpdateDiscount = useCallback(
		async (discount: ICreateDiscountRequest) => {
			if (!appId || !editingDiscount || !salesState) return;
			await salesState.updateDiscount(appId, editingDiscount.id, discount);
			toast.success(t("discountUpdated", "Discount updated"));
			loadData();
		},
		[appId, salesState, editingDiscount, loadData, t],
	);

	const handleToggleDiscount = useCallback(
		async (discountId: string) => {
			if (!appId || !salesState) return;
			await salesState.toggleDiscount(appId, discountId);
			loadData();
		},
		[appId, salesState, loadData],
	);

	const handleDeleteDiscount = useCallback(
		async (discountId: string) => {
			if (!appId || !salesState) return;
			await salesState.deleteDiscount(appId, discountId);
			toast.success(t("discountDeleted", "Discount deleted"));
			loadData();
		},
		[appId, salesState, loadData, t],
	);

	const handleUpdatePrice = useCallback(
		async (price: number) => {
			if (!appId || !salesState) return;
			await salesState.updatePrice(appId, price);
			loadData();
		},
		[appId, salesState, loadData],
	);

	const copyDiscountCode = useCallback(
		(code: string) => {
			navigator.clipboard.writeText(code);
			toast.success(
				t("copiedCodeToClipboard", 'Copied "{{code}}" to clipboard', { code }),
			);
		},
		[t],
	);

	const openDiscountDialog = useCallback((discount?: IDiscount) => {
		setEditingDiscount(discount);
		setDiscountDialogOpen(true);
	}, []);

	if (!salesState) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-muted-foreground">
					{t("salesTrackingIsNotAvailable", "Sales tracking is not available")}
				</p>
			</div>
		);
	}

	if (!appId) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-muted-foreground">
					{t("noAppSelected2", "No app selected")}
				</p>
			</div>
		);
	}

	// Ahead of the loading branch: `loadData` never runs without the Owner
	// check, so a denied role would otherwise sit on the skeleton forever — and
	// before that, on a revenue dashboard reading a confident €0.00.
	if (!canReadSales) {
		return (
			<SectionLockedPanel
				feature={t("monetization", "Monetization")}
				description={t(
					"onlyThisProjectsOwnerCanSeeItsRevenueAndPaymentSettings",
					"Only this project's owner can see its revenue, pricing and payment settings.",
				)}
				missing={[RolePermissions.Owner]}
				roleName={permissions.roleName}
			/>
		);
	}

	const busy = loading || permissions.isLoading;

	return (
		<div className="space-y-6">
			<div>
				<h1 className="text-2xl font-bold">
					{t("monetization", "Monetization")}
				</h1>
				<p className="text-muted-foreground">
					{t(
						"monetizationDescription",
						"Revenue from store sales and flow payments, plus pricing and payment settings",
					)}
				</p>
			</div>

			<Tabs value={activeTab} onValueChange={changeTab} className="space-y-6">
				<TabsList>
					<TabsTrigger value="overview">
						{t("overview", "Overview")}
					</TabsTrigger>
					<TabsTrigger value="pricing">
						{t("pricingAndDiscounts", "Pricing & discounts")}
					</TabsTrigger>
					{paymentsOn && (
						<TabsTrigger value="settings">
							{t("paymentSettings", "Payment settings")}
						</TabsTrigger>
					)}
				</TabsList>

				<TabsContent value="overview">
					{busy ? (
						<LoadingSkeleton />
					) : (
						<OverviewTab
							store={store}
							flows={flows}
							storeListed={storeListed}
							dateRange={dateRange}
							onDateRangeChange={setDateRange}
						/>
					)}
				</TabsContent>

				<TabsContent value="pricing">
					{busy ? (
						<LoadingSkeleton />
					) : (
						<PricingTab
							appId={appId}
							storeListed={storeListed}
							price={store?.overview.currentPrice ?? null}
							discounts={discounts}
							onEditPrice={() => setPriceDialogOpen(true)}
							onCreateDiscount={() => openDiscountDialog()}
							onEditDiscount={openDiscountDialog}
							onToggleDiscount={handleToggleDiscount}
							onDeleteDiscount={handleDeleteDiscount}
							onCopyCode={copyDiscountCode}
						/>
					)}
				</TabsContent>

				{paymentsOn && (
					<TabsContent value="settings">
						<AppPaymentSettingsPanel appId={appId} />
					</TabsContent>
				)}
			</Tabs>

			<DiscountDialog
				open={discountDialogOpen}
				onOpenChange={setDiscountDialogOpen}
				onSave={editingDiscount ? handleUpdateDiscount : handleCreateDiscount}
				discount={editingDiscount}
			/>

			<PriceEditorDialog
				open={priceDialogOpen}
				onOpenChange={setPriceDialogOpen}
				currentPrice={store?.overview.currentPrice ?? 0}
				onSave={handleUpdatePrice}
			/>
		</div>
	);
}
