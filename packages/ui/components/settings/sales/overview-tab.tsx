"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarIcon,
	EuroIcon,
	ShoppingCartIcon,
	StoreIcon,
	WorkflowIcon,
} from "lucide-react";
import type {
	IDailyStat,
	IFlowPaymentItem,
	IFlowPaymentsReport,
	IPurchaseItem,
	ISalesOverview,
} from "../../../state/backend-state/sales-state";
import { Badge } from "../../ui/badge";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import {
	PurchasesChart,
	RevenueBreakdownChart,
	RevenueChart,
} from "./sales-charts";
import { formatCurrency, formatDate } from "./sales-format";
import {
	CustomerCell,
	EmptyCard,
	SectionHeading,
	StatCard,
} from "./sales-parts";

export type DateRange = "7d" | "30d" | "90d";

export interface StoreSales {
	overview: ISalesOverview;
	dailyStats: IDailyStat[];
	purchases: IPurchaseItem[];
	purchaseTotal: number;
}

function DateRangeSelect({
	value,
	onChange,
}: {
	value: DateRange;
	onChange: (value: DateRange) => void;
}) {
	const { t } = useTranslation("settings");
	return (
		<Select value={value} onValueChange={(v) => onChange(v as DateRange)}>
			<SelectTrigger className="w-36">
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				<SelectItem value="7d">{t("last7Days", "Last 7 days")}</SelectItem>
				<SelectItem value="30d">{t("last30Days", "Last 30 days")}</SelectItem>
				<SelectItem value="90d">{t("last90Days", "Last 90 days")}</SelectItem>
			</SelectContent>
		</Select>
	);
}

function RevenueSummary({
	store,
	flows,
}: {
	store: StoreSales | null;
	flows: IFlowPaymentsReport | null;
}) {
	const { t } = useTranslation("settings");
	const both = !!store && !!flows;
	const split = (storeCents: number, flowCents: number) =>
		both
			? t("storeAndFlowsSplit", "Store {{store}} · Flows {{flows}}", {
					store: formatCurrency(storeCents),
					flows: formatCurrency(flowCents),
				})
			: undefined;
	const storeTotal = store?.overview.totalRevenue ?? 0;
	const flowTotal = flows?.totalRevenue ?? 0;
	const storePeriod = store?.overview.periodRevenue ?? 0;
	const flowPeriod = flows?.periodRevenue ?? 0;

	return (
		<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
			<StatCard
				title={t("totalRevenue", "Total Revenue")}
				value={formatCurrency(storeTotal + flowTotal)}
				subtitle={split(storeTotal, flowTotal)}
				icon={EuroIcon}
			/>
			<StatCard
				title={t("revenueLast30Days", "Revenue, last 30 days")}
				value={formatCurrency(storePeriod + flowPeriod)}
				change={
					both
						? undefined
						: (store?.overview.revenueChangePercent ??
							flows?.revenueChangePercent)
				}
				subtitle={split(storePeriod, flowPeriod)}
				icon={CalendarIcon}
			/>
			{store && (
				<StatCard
					title={t("storePurchases", "Store purchases")}
					value={store.overview.totalPurchases.toString()}
					change={store.overview.purchasesChangePercent}
					subtitle={t(
						"buyersAndAverageOrder",
						"{{buyers}} buyers · avg. {{average}}",
						{
							buyers: store.overview.uniqueBuyers,
							average: formatCurrency(store.overview.avgOrderValue),
						},
					)}
					icon={ShoppingCartIcon}
				/>
			)}
			{flows && (
				<StatCard
					title={t("flowPayments", "Flow payments")}
					value={flows.totalPayments.toString()}
					change={flows.paymentsChangePercent}
					subtitle={t(
						"payersAndRefunds",
						"{{payers}} payers · {{refunded}} refunded",
						{
							payers: flows.uniquePayers,
							refunded: formatCurrency(flows.totalRefunded),
						},
					)}
					icon={WorkflowIcon}
				/>
			)}
		</div>
	);
}

function RevenueCharts({
	store,
	flows,
}: {
	store: StoreSales | null;
	flows: IFlowPaymentsReport | null;
}) {
	const { t } = useTranslation("settings");
	return (
		<Tabs defaultValue="revenue" className="space-y-4">
			{store && (
				<TabsList>
					<TabsTrigger value="revenue">{t("revenue", "Revenue")}</TabsTrigger>
					<TabsTrigger value="purchases">
						{t("purchases", "Purchases")}
					</TabsTrigger>
					<TabsTrigger value="breakdown">
						{t("breakdown", "Breakdown")}
					</TabsTrigger>
				</TabsList>
			)}

			<TabsContent value="revenue">
				<Card>
					<CardHeader>
						<CardTitle>{t("revenueOverTime", "Revenue Over Time")}</CardTitle>
						<CardDescription>
							{store && flows
								? t(
										"dailyStoreAndFlowRevenue",
										"Daily store sales and flow payments for the selected period",
									)
								: t(
										"dailyRevenueForTheSelectedPeriod",
										"Daily revenue for the selected period",
									)}
						</CardDescription>
					</CardHeader>
					<CardContent>
						<RevenueChart store={store?.dailyStats} flows={flows?.dailyStats} />
					</CardContent>
				</Card>
			</TabsContent>

			{store && (
				<>
					<TabsContent value="purchases">
						<Card>
							<CardHeader>
								<CardTitle>
									{t("purchasesRefunds", "Purchases & Refunds")}
								</CardTitle>
								<CardDescription>
									{t(
										"dailyPurchaseAndRefundActivity",
										"Daily purchase and refund activity",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<PurchasesChart data={store.dailyStats} />
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="breakdown">
						<Card>
							<CardHeader>
								<CardTitle>
									{t("revenueBreakdown", "Revenue Breakdown")}
								</CardTitle>
								<CardDescription>
									{t(
										"netRevenueVsRefundsAndDiscounts",
										"Net revenue vs refunds and discounts",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<RevenueBreakdownChart data={store.overview} />
							</CardContent>
						</Card>
					</TabsContent>
				</>
			)}
		</Tabs>
	);
}

function RecentPurchases({ store }: { store: StoreSales }) {
	const { t } = useTranslation("settings");
	return (
		<div className="space-y-4">
			<SectionHeading
				title={t("recentStorePurchases", "Recent store purchases")}
				description={t(
					"purchasetotalTotalPurchases",
					"{{purchaseTotal}} total purchases",
					{ purchaseTotal: store.purchaseTotal },
				)}
			/>
			{store.purchases.length === 0 ? (
				<EmptyCard
					icon={ShoppingCartIcon}
					message={t("noPurchasesYet", "No purchases yet")}
				/>
			) : (
				<Card>
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>{t("user", "User")}</TableHead>
								<TableHead>{t("amount", "Amount")}</TableHead>
								<TableHead>{t("discount", "Discount")}</TableHead>
								<TableHead>{t("status", "Status")}</TableHead>
								<TableHead>{t("date", "Date")}</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{store.purchases.map((purchase) => (
								<TableRow key={purchase.id}>
									<TableCell>
										<CustomerCell
											name={purchase.userName}
											avatar={purchase.userAvatar}
											fallback={purchase.userId.slice(0, 8)}
										/>
									</TableCell>
									<TableCell>{formatCurrency(purchase.pricePaid)}</TableCell>
									<TableCell>
										{purchase.discountAmount > 0 ? (
											<Badge variant="secondary">
												-{formatCurrency(purchase.discountAmount)}
											</Badge>
										) : (
											<span className="text-muted-foreground">-</span>
										)}
									</TableCell>
									<TableCell>
										<Badge
											variant={
												purchase.status === "Completed"
													? "default"
													: purchase.status === "Refunded"
														? "destructive"
														: "secondary"
											}
										>
											{purchase.status}
										</Badge>
									</TableCell>
									<TableCell>
										{purchase.completedAt
											? formatDate(purchase.completedAt)
											: "-"}
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				</Card>
			)}
		</div>
	);
}

function FlowPaymentStatus({ payment }: { payment: IFlowPaymentItem }) {
	const { t } = useTranslation("settings");
	if (payment.refunded > 0 && payment.refunded >= payment.collected) {
		return <Badge variant="destructive">{t("refunded", "Refunded")}</Badge>;
	}
	if (payment.refunded > 0) {
		return (
			<Badge variant="secondary">
				{t("partiallyRefunded", "Partially refunded")}
			</Badge>
		);
	}
	return <Badge variant="default">{t("paid", "Paid")}</Badge>;
}

function RecentFlowPayments({ flows }: { flows: IFlowPaymentsReport }) {
	const { t } = useTranslation("settings");
	return (
		<div className="space-y-4">
			<SectionHeading
				title={t("recentFlowPayments", "Recent flow payments")}
				description={t(
					"flowPaymentsDescription",
					"Payments your flows collected with a payment node, before payment provider and platform fees",
				)}
			/>
			{flows.recentPayments.length === 0 ? (
				<EmptyCard
					icon={WorkflowIcon}
					message={t(
						"noFlowPaymentsYet",
						"No flow payments yet. Add a payment node to a flow to charge users while it runs.",
					)}
				/>
			) : (
				<Card>
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>{t("product", "Product")}</TableHead>
								<TableHead>{t("payer", "Payer")}</TableHead>
								<TableHead>{t("amount", "Amount")}</TableHead>
								<TableHead>{t("status", "Status")}</TableHead>
								<TableHead>{t("date", "Date")}</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{flows.recentPayments.map((payment) => (
								<TableRow key={payment.id}>
									<TableCell>
										<div className="flex flex-col">
											<span>{payment.productName ?? "-"}</span>
											{payment.reference && (
												<span className="text-xs text-muted-foreground">
													{payment.reference}
												</span>
											)}
										</div>
									</TableCell>
									<TableCell>
										<CustomerCell
											name={payment.payerName}
											avatar={payment.payerAvatar}
											fallback={payment.payerUserId?.slice(0, 8) ?? "-"}
										/>
									</TableCell>
									<TableCell>
										<div className="flex flex-col">
											<span>{formatCurrency(payment.collected)}</span>
											{payment.refunded > 0 && (
												<span className="text-xs text-muted-foreground">
													-{formatCurrency(payment.refunded)}
												</span>
											)}
										</div>
									</TableCell>
									<TableCell>
										<FlowPaymentStatus payment={payment} />
									</TableCell>
									<TableCell>{formatDate(payment.createdAt)}</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				</Card>
			)}
		</div>
	);
}

export function OverviewTab({
	store,
	flows,
	storeListed,
	dateRange,
	onDateRangeChange,
}: {
	store: StoreSales | null;
	flows: IFlowPaymentsReport | null;
	storeListed: boolean;
	dateRange: DateRange;
	onDateRangeChange: (value: DateRange) => void;
}) {
	const { t } = useTranslation("settings");

	if (!store && !flows) {
		return (
			<EmptyCard
				icon={StoreIcon}
				message={
					storeListed
						? t(
								"revenueUnavailable",
								"Revenue could not be loaded. Try again later.",
							)
						: t(
								"noRevenueSourceYet",
								"Nothing earns money here yet. Make the app public to sell it in the store.",
							)
				}
			/>
		);
	}

	return (
		<div className="space-y-6">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<p className="text-sm text-muted-foreground">
					{t(
						"revenueBeforeFees",
						"Amounts are what buyers paid, before payment provider and platform fees.",
					)}
				</p>
				<DateRangeSelect value={dateRange} onChange={onDateRangeChange} />
			</div>

			<RevenueSummary store={store} flows={flows} />
			<RevenueCharts store={store} flows={flows} />

			{store ? (
				<RecentPurchases store={store} />
			) : (
				!storeListed && (
					<p className="rounded-lg border p-4 text-sm text-muted-foreground">
						{t(
							"storeSalesNeedPublicApp",
							"Store sales appear here once the app is public and has a price.",
						)}
					</p>
				)
			)}
			{flows && <RecentFlowPayments flows={flows} />}
		</div>
	);
}
