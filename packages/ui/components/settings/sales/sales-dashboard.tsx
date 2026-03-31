"use client";

import { ResponsiveBar } from "@nivo/bar";
import { ResponsiveLine } from "@nivo/line";
import { ResponsivePie } from "@nivo/pie";
import {
	ArrowDownIcon,
	ArrowUpIcon,
	CopyIcon,
	DollarSignIcon,
	PercentIcon,
	PlusIcon,
	ShoppingCartIcon,
	TagIcon,
	TrendingUpIcon,
	UsersIcon,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

import { useBackend } from "../../../state/backend-state";
import type {
	ICreateDiscountRequest,
	IDailyStat,
	IDiscount,
	IPurchaseItem,
	ISalesOverview,
} from "../../../state/backend-state/sales-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Separator } from "../../ui/separator";
import { Skeleton } from "../../ui/skeleton";
import { Switch } from "../../ui/switch";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import { Textarea } from "../../ui/textarea";

function formatCurrency(cents: number): string {
	return new Intl.NumberFormat("en-US", {
		style: "currency",
		currency: "EUR",
	}).format(cents / 100);
}

function formatPercent(value: number | null): string {
	if (value === null) return "N/A";
	return `${value >= 0 ? "+" : ""}${value.toFixed(1)}%`;
}

function formatDate(dateStr: string): string {
	return new Date(dateStr).toLocaleDateString("en-US", {
		month: "short",
		day: "numeric",
		year: "numeric",
	});
}

// Stat Card Component
function StatCard({
	title,
	value,
	change,
	icon: Icon,
	subtitle,
}: {
	title: string;
	value: string;
	change?: number | null;
	icon: React.ComponentType<{ className?: string }>;
	subtitle?: string;
}) {
	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
				<CardTitle className="text-sm font-medium">{title}</CardTitle>
				<Icon className="h-4 w-4 text-muted-foreground" />
			</CardHeader>
			<CardContent>
				<div className="text-2xl font-bold">{value}</div>
				{change !== undefined && change !== null && (
					<p
						className={`text-xs ${change >= 0 ? "text-green-600" : "text-red-600"} flex items-center gap-1`}
					>
						{change >= 0 ? (
							<ArrowUpIcon className="h-3 w-3" />
						) : (
							<ArrowDownIcon className="h-3 w-3" />
						)}
						{formatPercent(change)} from last period
					</p>
				)}
				{subtitle && (
					<p className="text-xs text-muted-foreground mt-1">{subtitle}</p>
				)}
			</CardContent>
		</Card>
	);
}

// Revenue Chart Component
function RevenueChart({ data }: { data: IDailyStat[] }) {
	const chartData = useMemo(
		() => [
			{
				id: "Revenue",
				color: "hsl(var(--primary))",
				data: data.map((d) => ({
					x: new Date(d.date).toLocaleDateString("en-US", {
						month: "short",
						day: "numeric",
					}),
					y: d.revenue / 100,
				})),
			},
		],
		[data],
	);

	if (data.length === 0) {
		return (
			<div className="h-[300px] flex items-center justify-center text-muted-foreground">
				No revenue data available
			</div>
		);
	}

	return (
		<div className="h-[300px]">
			<ResponsiveLine
				data={chartData}
				margin={{ top: 20, right: 20, bottom: 50, left: 60 }}
				xScale={{ type: "point" }}
				yScale={{ type: "linear", min: 0, max: "auto" }}
				axisBottom={{
					tickRotation: -45,
					legend: "Date",
					legendOffset: 40,
					legendPosition: "middle",
				}}
				axisLeft={{
					legend: "Revenue (€)",
					legendOffset: -50,
					legendPosition: "middle",
					format: (v) => `€${v}`,
				}}
				colors={{ scheme: "category10" }}
				pointSize={8}
				pointBorderWidth={2}
				pointBorderColor={{ from: "serieColor" }}
				enableArea={true}
				areaOpacity={0.1}
				useMesh={true}
				enableGridX={false}
				theme={{
					axis: {
						ticks: { text: { fill: "hsl(var(--muted-foreground))" } },
						legend: { text: { fill: "hsl(var(--muted-foreground))" } },
					},
					grid: { line: { stroke: "hsl(var(--border))" } },
					crosshair: { line: { stroke: "hsl(var(--primary))" } },
				}}
			/>
		</div>
	);
}

// Purchases by Day Chart
function PurchasesChart({ data }: { data: IDailyStat[] }) {
	const chartData = useMemo(
		() =>
			data.map((d) => ({
				date: new Date(d.date).toLocaleDateString("en-US", {
					month: "short",
					day: "numeric",
				}),
				purchases: d.purchases,
				refunds: d.refunds,
			})),
		[data],
	);

	if (data.length === 0) {
		return (
			<div className="h-[300px] flex items-center justify-center text-muted-foreground">
				No purchase data available
			</div>
		);
	}

	return (
		<div className="h-[300px]">
			<ResponsiveBar
				data={chartData}
				keys={["purchases", "refunds"]}
				indexBy="date"
				margin={{ top: 20, right: 100, bottom: 50, left: 60 }}
				padding={0.3}
				groupMode="grouped"
				colors={{ scheme: "paired" }}
				axisBottom={{
					tickRotation: -45,
					legend: "Date",
					legendOffset: 40,
					legendPosition: "middle",
				}}
				axisLeft={{
					legend: "Count",
					legendOffset: -50,
					legendPosition: "middle",
				}}
				legends={[
					{
						dataFrom: "keys",
						anchor: "bottom-right",
						direction: "column",
						translateX: 100,
						itemWidth: 80,
						itemHeight: 20,
						itemTextColor: "hsl(var(--muted-foreground))",
					},
				]}
				theme={{
					axis: {
						ticks: { text: { fill: "hsl(var(--muted-foreground))" } },
						legend: { text: { fill: "hsl(var(--muted-foreground))" } },
					},
					grid: { line: { stroke: "hsl(var(--border))" } },
					legends: { text: { fill: "hsl(var(--muted-foreground))" } },
				}}
			/>
		</div>
	);
}

// Revenue Breakdown Pie Chart
function RevenueBreakdownChart({ data }: { data: ISalesOverview }) {
	const chartData = useMemo(
		() =>
			[
				{
					id: "Net Revenue",
					value: data.netRevenue / 100,
					color: "hsl(142, 76%, 36%)",
				},
				{
					id: "Refunds",
					value: data.refundAmount / 100,
					color: "hsl(0, 84%, 60%)",
				},
				{
					id: "Discounts",
					value: data.totalDiscounts / 100,
					color: "hsl(45, 93%, 47%)",
				},
			].filter((d) => d.value > 0),
		[data],
	);

	if (chartData.length === 0) {
		return (
			<div className="h-[250px] flex items-center justify-center text-muted-foreground">
				No revenue breakdown available
			</div>
		);
	}

	return (
		<div className="h-[250px]">
			<ResponsivePie
				data={chartData}
				margin={{ top: 20, right: 80, bottom: 20, left: 80 }}
				innerRadius={0.5}
				padAngle={0.7}
				cornerRadius={3}
				activeOuterRadiusOffset={8}
				colors={{ datum: "data.color" }}
				arcLinkLabelsSkipAngle={10}
				arcLinkLabelsTextColor="hsl(var(--muted-foreground))"
				arcLinkLabelsThickness={2}
				arcLabelsSkipAngle={10}
				arcLabelsTextColor="white"
				valueFormat={(v) => `€${v.toFixed(0)}`}
				theme={{
					labels: { text: { fill: "hsl(var(--foreground))" } },
				}}
			/>
		</div>
	);
}

// Discount Management Dialog
function DiscountDialog({
	open,
	onOpenChange,
	onSave,
	discount,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSave: (discount: ICreateDiscountRequest) => Promise<void>;
	discount?: IDiscount;
}) {
	const [form, setForm] = useState<ICreateDiscountRequest>({
		code: "",
		name: "",
		description: "",
		discountType: "percentage",
		discountValue: 10,
		maxUses: undefined,
		minPurchaseAmount: undefined,
		startsAt: undefined,
		expiresAt: undefined,
	});
	const [saving, setSaving] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: Reset form when dialog opens/closes
	useEffect(() => {
		if (discount) {
			setForm({
				code: discount.code,
				name: discount.name,
				description: discount.description || "",
				discountType:
					discount.discountType === "Percentage"
						? "percentage"
						: "fixed_amount",
				discountValue: discount.discountValue,
				maxUses: discount.maxUses || undefined,
				minPurchaseAmount: discount.minPurchaseAmount || undefined,
				startsAt: discount.startsAt,
				expiresAt: discount.expiresAt || undefined,
			});
		} else {
			setForm({
				code: "",
				name: "",
				description: "",
				discountType: "percentage",
				discountValue: 10,
				maxUses: undefined,
				minPurchaseAmount: undefined,
				startsAt: undefined,
				expiresAt: undefined,
			});
		}
	}, [discount, open]);

	const handleSave = useCallback(async () => {
		setSaving(true);
		try {
			await onSave(form);
			onOpenChange(false);
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : "Failed to save discount",
			);
		} finally {
			setSaving(false);
		}
	}, [form, onSave, onOpenChange]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-md">
				<DialogHeader>
					<DialogTitle>
						{discount ? "Edit Discount" : "Create Discount"}
					</DialogTitle>
					<DialogDescription>
						{discount
							? "Update the discount details"
							: "Create a new discount code for your app"}
					</DialogDescription>
				</DialogHeader>

				<div className="grid gap-4 py-4">
					<div className="grid gap-2">
						<Label htmlFor="code">Code</Label>
						<Input
							id="code"
							placeholder="LAUNCH20"
							value={form.code}
							onChange={(e) =>
								setForm({ ...form, code: e.target.value.toUpperCase() })
							}
						/>
					</div>

					<div className="grid gap-2">
						<Label htmlFor="name">Name</Label>
						<Input
							id="name"
							placeholder="Launch Discount"
							value={form.name}
							onChange={(e) => setForm({ ...form, name: e.target.value })}
						/>
					</div>

					<div className="grid gap-2">
						<Label htmlFor="description">Description</Label>
						<Textarea
							id="description"
							placeholder="Special launch offer..."
							value={form.description}
							onChange={(e) =>
								setForm({ ...form, description: e.target.value })
							}
						/>
					</div>

					<div className="grid grid-cols-2 gap-4">
						<div className="grid gap-2">
							<Label>Type</Label>
							<Select
								value={form.discountType}
								onValueChange={(v) =>
									setForm({
										...form,
										discountType: v as "percentage" | "fixed_amount",
									})
								}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="percentage">Percentage</SelectItem>
									<SelectItem value="fixed_amount">Fixed Amount</SelectItem>
								</SelectContent>
							</Select>
						</div>

						<div className="grid gap-2">
							<Label htmlFor="value">
								Value {form.discountType === "percentage" ? "(%)" : "(cents)"}
							</Label>
							<Input
								id="value"
								type="number"
								min={0}
								max={form.discountType === "percentage" ? 100 : undefined}
								value={form.discountValue}
								onChange={(e) =>
									setForm({
										...form,
										discountValue: Number.parseInt(e.target.value) || 0,
									})
								}
							/>
						</div>
					</div>

					<div className="grid grid-cols-2 gap-4">
						<div className="grid gap-2">
							<Label htmlFor="maxUses">Max Uses (optional)</Label>
							<Input
								id="maxUses"
								type="number"
								min={1}
								placeholder="Unlimited"
								value={form.maxUses || ""}
								onChange={(e) =>
									setForm({
										...form,
										maxUses: e.target.value
											? Number.parseInt(e.target.value)
											: undefined,
									})
								}
							/>
						</div>

						<div className="grid gap-2">
							<Label htmlFor="minAmount">Min Amount (cents)</Label>
							<Input
								id="minAmount"
								type="number"
								min={0}
								placeholder="No minimum"
								value={form.minPurchaseAmount || ""}
								onChange={(e) =>
									setForm({
										...form,
										minPurchaseAmount: e.target.value
											? Number.parseInt(e.target.value)
											: undefined,
									})
								}
							/>
						</div>
					</div>
				</div>

				<DialogFooter>
					<Button variant="outline" onClick={() => onOpenChange(false)}>
						Cancel
					</Button>
					<Button
						onClick={handleSave}
						disabled={saving || !form.code || !form.name}
					>
						{saving ? "Saving..." : discount ? "Update" : "Create"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

// Price Editor Dialog
function PriceEditorDialog({
	open,
	onOpenChange,
	currentPrice,
	onSave,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	currentPrice: number;
	onSave: (price: number) => Promise<void>;
}) {
	const [price, setPrice] = useState(currentPrice);
	const [saving, setSaving] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: Reset price when dialog opens/closes
	useEffect(() => {
		setPrice(currentPrice);
	}, [currentPrice, open]);

	const handleSave = useCallback(async () => {
		setSaving(true);
		try {
			await onSave(price);
			onOpenChange(false);
			toast.success("Price updated successfully");
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : "Failed to update price",
			);
		} finally {
			setSaving(false);
		}
	}, [price, onSave, onOpenChange]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-sm">
				<DialogHeader>
					<DialogTitle>Update Price</DialogTitle>
					<DialogDescription>
						Set a new price for your app (in cents)
					</DialogDescription>
				</DialogHeader>

				<div className="grid gap-4 py-4">
					<div className="grid gap-2">
						<Label htmlFor="price">Price (cents)</Label>
						<div className="flex items-center gap-2">
							<Input
								id="price"
								type="number"
								min={0}
								value={price}
								onChange={(e) => setPrice(Number.parseInt(e.target.value) || 0)}
							/>
							<span className="text-muted-foreground whitespace-nowrap">
								= {formatCurrency(price)}
							</span>
						</div>
					</div>
				</div>

				<DialogFooter>
					<Button variant="outline" onClick={() => onOpenChange(false)}>
						Cancel
					</Button>
					<Button onClick={handleSave} disabled={saving}>
						{saving ? "Saving..." : "Update Price"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function SalesDashboard() {
	const backend = useBackend();
	const salesState = backend.salesState;
	const searchParams = useSearchParams();
	const appId = searchParams.get("id");

	const [loading, setLoading] = useState(true);
	const [overview, setOverview] = useState<ISalesOverview | null>(null);
	const [dailyStats, setDailyStats] = useState<IDailyStat[]>([]);
	const [purchases, setPurchases] = useState<IPurchaseItem[]>([]);
	const [discounts, setDiscounts] = useState<IDiscount[]>([]);
	const [purchaseTotal, setPurchaseTotal] = useState(0);

	const [discountDialogOpen, setDiscountDialogOpen] = useState(false);
	const [editingDiscount, setEditingDiscount] = useState<
		IDiscount | undefined
	>();
	const [priceDialogOpen, setPriceDialogOpen] = useState(false);

	const [dateRange, setDateRange] = useState<"7d" | "30d" | "90d">("30d");

	// Load data
	const loadData = useCallback(async () => {
		if (!appId || !salesState) return;

		setLoading(true);
		try {
			const days = dateRange === "7d" ? 7 : dateRange === "30d" ? 30 : 90;
			const endDate = new Date().toISOString().split("T")[0];
			const startDate = new Date(Date.now() - days * 24 * 60 * 60 * 1000)
				.toISOString()
				.split("T")[0];

			const [overviewData, statsData, purchasesData, discountsData] =
				await Promise.all([
					salesState.getSalesOverview(appId),
					salesState.getSalesStats(appId, startDate, endDate),
					salesState.listPurchases(appId, undefined, 0, 50),
					salesState.listDiscounts(appId),
				]);

			setOverview(overviewData);
			setDailyStats(statsData.dailyStats);
			setPurchases(purchasesData.purchases);
			setPurchaseTotal(purchasesData.total);
			setDiscounts(discountsData);
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : "Failed to load sales data",
			);
		} finally {
			setLoading(false);
		}
	}, [appId, dateRange, salesState]);

	useEffect(() => {
		loadData();
	}, [loadData]);

	// Discount handlers
	const handleCreateDiscount = useCallback(
		async (discount: ICreateDiscountRequest) => {
			if (!appId || !salesState) return;
			await salesState.createDiscount(appId, discount);
			toast.success("Discount created");
			loadData();
		},
		[appId, salesState, loadData],
	);

	const handleUpdateDiscount = useCallback(
		async (discount: ICreateDiscountRequest) => {
			if (!appId || !editingDiscount || !salesState) return;
			await salesState.updateDiscount(appId, editingDiscount.id, discount);
			toast.success("Discount updated");
			loadData();
		},
		[appId, salesState, editingDiscount, loadData],
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
			toast.success("Discount deleted");
			loadData();
		},
		[appId, salesState, loadData],
	);

	const handleUpdatePrice = useCallback(
		async (price: number) => {
			if (!appId || !salesState) return;
			await salesState.updatePrice(appId, price);
			loadData();
		},
		[appId, salesState, loadData],
	);

	const copyDiscountCode = useCallback((code: string) => {
		navigator.clipboard.writeText(code);
		toast.success(`Copied "${code}" to clipboard`);
	}, []);

	if (!salesState) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-muted-foreground">Sales tracking is not available</p>
			</div>
		);
	}

	if (!appId) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-muted-foreground">No app selected</p>
			</div>
		);
	}

	if (loading) {
		return (
			<div className="p-6 space-y-6">
				<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
					<Skeleton key="skeleton-1" className="h-32" />
					<Skeleton key="skeleton-2" className="h-32" />
					<Skeleton key="skeleton-3" className="h-32" />
					<Skeleton key="skeleton-4" className="h-32" />
				</div>
				<Skeleton className="h-[300px]" />
			</div>
		);
	}

	return (
		<div className="space-y-6">
				{/* Header */}
				<div className="flex items-center justify-between">
					<div>
						<h1 className="text-2xl font-bold">Sales Dashboard</h1>
						<p className="text-muted-foreground">
							Track your app&apos;s revenue and manage pricing
						</p>
					</div>
					<div className="flex items-center gap-2">
						<Select
							value={dateRange}
							onValueChange={(v) => setDateRange(v as typeof dateRange)}
						>
							<SelectTrigger className="w-32">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="7d">Last 7 days</SelectItem>
								<SelectItem value="30d">Last 30 days</SelectItem>
								<SelectItem value="90d">Last 90 days</SelectItem>
							</SelectContent>
						</Select>
						<Button variant="outline" onClick={() => setPriceDialogOpen(true)}>
							<DollarSignIcon className="h-4 w-4 mr-2" />
							{overview ? formatCurrency(overview.currentPrice) : "Set Price"}
						</Button>
					</div>
				</div>

				{/* Stats Cards */}
				<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
					<StatCard
						title="Total Revenue"
						value={formatCurrency(overview?.totalRevenue ?? 0)}
						change={overview?.revenueChangePercent}
						icon={DollarSignIcon}
					/>
					<StatCard
						title="Total Purchases"
						value={(overview?.totalPurchases ?? 0).toString()}
						change={overview?.purchasesChangePercent}
						icon={ShoppingCartIcon}
					/>
					<StatCard
						title="Unique Buyers"
						value={(overview?.uniqueBuyers ?? 0).toString()}
						icon={UsersIcon}
					/>
					<StatCard
						title="Avg. Order Value"
						value={formatCurrency(overview?.avgOrderValue ?? 0)}
						icon={TrendingUpIcon}
					/>
				</div>

				{/* Charts */}
				<Tabs defaultValue="revenue" className="space-y-4">
					<TabsList>
						<TabsTrigger value="revenue">Revenue</TabsTrigger>
						<TabsTrigger value="purchases">Purchases</TabsTrigger>
						<TabsTrigger value="breakdown">Breakdown</TabsTrigger>
					</TabsList>

					<TabsContent value="revenue">
						<Card>
							<CardHeader>
								<CardTitle>Revenue Over Time</CardTitle>
								<CardDescription>
									Daily revenue for the selected period
								</CardDescription>
							</CardHeader>
							<CardContent>
								<RevenueChart data={dailyStats} />
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="purchases">
						<Card>
							<CardHeader>
								<CardTitle>Purchases & Refunds</CardTitle>
								<CardDescription>
									Daily purchase and refund activity
								</CardDescription>
							</CardHeader>
							<CardContent>
								<PurchasesChart data={dailyStats} />
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="breakdown">
						<Card>
							<CardHeader>
								<CardTitle>Revenue Breakdown</CardTitle>
								<CardDescription>
									Net revenue vs refunds and discounts
								</CardDescription>
							</CardHeader>
							<CardContent>
								{overview && <RevenueBreakdownChart data={overview} />}
							</CardContent>
						</Card>
					</TabsContent>
				</Tabs>

				<Separator />

				{/* Discounts Section */}
				<div className="space-y-4">
					<div className="flex items-center justify-between">
						<div>
							<h2 className="text-xl font-semibold">Discount Codes</h2>
							<p className="text-sm text-muted-foreground">
								Manage promotional discounts for your app
							</p>
						</div>
						<Button
							onClick={() => {
								setEditingDiscount(undefined);
								setDiscountDialogOpen(true);
							}}
						>
							<PlusIcon className="h-4 w-4 mr-2" />
							Add Discount
						</Button>
					</div>

					{discounts.length === 0 ? (
						<Card>
							<CardContent className="flex flex-col items-center justify-center py-12">
								<TagIcon className="h-12 w-12 text-muted-foreground mb-4" />
								<p className="text-muted-foreground">
									No discounts created yet
								</p>
								<Button
									variant="outline"
									className="mt-4"
									onClick={() => {
										setEditingDiscount(undefined);
										setDiscountDialogOpen(true);
									}}
								>
									Create your first discount
								</Button>
							</CardContent>
						</Card>
					) : (
						<Card>
							<Table>
								<TableHeader>
									<TableRow>
										<TableHead>Code</TableHead>
										<TableHead>Name</TableHead>
										<TableHead>Discount</TableHead>
										<TableHead>Uses</TableHead>
										<TableHead>Status</TableHead>
										<TableHead className="text-right">Actions</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{discounts.map((discount) => (
										<TableRow key={discount.id}>
											<TableCell>
												<div className="flex items-center gap-2">
													<code className="bg-muted px-2 py-1 rounded text-sm">
														{discount.code}
													</code>
													<Button
														variant="ghost"
														size="icon"
														className="h-6 w-6"
														onClick={() => copyDiscountCode(discount.code)}
													>
														<CopyIcon className="h-3 w-3" />
													</Button>
												</div>
											</TableCell>
											<TableCell>{discount.name}</TableCell>
											<TableCell>
												{discount.discountType === "Percentage" ? (
													<span className="flex items-center gap-1">
														<PercentIcon className="h-3 w-3" />
														{discount.discountValue}%
													</span>
												) : (
													formatCurrency(discount.discountValue)
												)}
											</TableCell>
											<TableCell>
												{discount.usedCount}
												{discount.maxUses && ` / ${discount.maxUses}`}
											</TableCell>
											<TableCell>
												<div className="flex items-center gap-2">
													<Switch
														checked={discount.isActive}
														onCheckedChange={() =>
															handleToggleDiscount(discount.id)
														}
													/>
													{discount.isValid ? (
														<Badge variant="default">Active</Badge>
													) : discount.isActive ? (
														<Badge variant="secondary">Expired</Badge>
													) : (
														<Badge variant="outline">Disabled</Badge>
													)}
												</div>
											</TableCell>
											<TableCell className="text-right">
												<div className="flex justify-end gap-2">
													<Button
														variant="ghost"
														size="sm"
														onClick={() => {
															setEditingDiscount(discount);
															setDiscountDialogOpen(true);
														}}
													>
														Edit
													</Button>
													<Button
														variant="ghost"
														size="sm"
														className="text-destructive"
														onClick={() => handleDeleteDiscount(discount.id)}
													>
														Delete
													</Button>
												</div>
											</TableCell>
										</TableRow>
									))}
								</TableBody>
							</Table>
						</Card>
					)}
				</div>

				<Separator />

				{/* Recent Purchases */}
				<div className="space-y-4">
					<div>
						<h2 className="text-xl font-semibold">Recent Purchases</h2>
						<p className="text-sm text-muted-foreground">
							{purchaseTotal} total purchases
						</p>
					</div>

					{purchases.length === 0 ? (
						<Card>
							<CardContent className="flex flex-col items-center justify-center py-12">
								<ShoppingCartIcon className="h-12 w-12 text-muted-foreground mb-4" />
								<p className="text-muted-foreground">No purchases yet</p>
							</CardContent>
						</Card>
					) : (
						<Card>
							<Table>
								<TableHeader>
									<TableRow>
										<TableHead>User</TableHead>
										<TableHead>Amount</TableHead>
										<TableHead>Discount</TableHead>
										<TableHead>Status</TableHead>
										<TableHead>Date</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{purchases.map((purchase) => (
										<TableRow key={purchase.id}>
											<TableCell>
												<div className="flex items-center gap-2">
													{purchase.userAvatar && (
														<img
															src={purchase.userAvatar}
															alt=""
															className="h-6 w-6 rounded-full"
														/>
													)}
													<span>
														{purchase.userName || purchase.userId.slice(0, 8)}
													</span>
												</div>
											</TableCell>
											<TableCell>
												{formatCurrency(purchase.pricePaid)}
											</TableCell>
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

			{/* Dialogs */}
			<DiscountDialog
				open={discountDialogOpen}
				onOpenChange={setDiscountDialogOpen}
				onSave={editingDiscount ? handleUpdateDiscount : handleCreateDiscount}
				discount={editingDiscount}
			/>

			<PriceEditorDialog
				open={priceDialogOpen}
				onOpenChange={setPriceDialogOpen}
				currentPrice={overview?.currentPrice ?? 0}
				onSave={handleUpdatePrice}
			/>
		</div>
	);
}
