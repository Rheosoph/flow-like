/**
 * Sales dashboard state interface
 * Used for managing app sales, discounts, and analytics
 */

export interface ISalesOverview {
	totalRevenue: number;
	totalPurchases: number;
	totalRefunds: number;
	refundAmount: number;
	netRevenue: number;
	uniqueBuyers: number;
	avgOrderValue: number;
	currentPrice: number;
	totalDiscounts: number;
	totalMembers: number;
	periodRevenue: number;
	periodPurchases: number;
	revenueChangePercent: number | null;
	purchasesChangePercent: number | null;
}

export interface IDailyStat {
	date: string;
	revenue: number;
	grossRevenue: number;
	discounts: number;
	purchases: number;
	refunds: number;
	refundAmount: number;
	uniqueBuyers: number;
	avgOrderValue: number;
}

export interface ISalesStats {
	dailyStats: IDailyStat[];
	summary: ISalesOverview;
}

export interface IPurchaseItem {
	id: string;
	userId: string;
	userName: string | null;
	userAvatar: string | null;
	pricePaid: number;
	originalPrice: number;
	discountAmount: number;
	discountId: string | null;
	currency: string;
	status: string;
	completedAt: string | null;
	refundedAt: string | null;
	refundReason: string | null;
	createdAt: string;
}

export interface IPurchasesResponse {
	purchases: IPurchaseItem[];
	total: number;
	offset: number;
	limit: number;
}

export interface IFlowPaymentDay {
	date: string;
	revenue: number;
	collected: number;
	refunded: number;
	payments: number;
}

export interface IFlowPaymentItem {
	id: string;
	productName: string | null;
	reference: string | null;
	payerUserId: string | null;
	payerName: string | null;
	payerAvatar: string | null;
	boardId: string | null;
	runId: string | null;
	amount: number;
	collected: number;
	refunded: number;
	currency: string;
	/** Unix milliseconds */
	createdAt: number;
}

/** Money the app's flows collected through payment nodes, in cents. */
export interface IFlowPaymentsReport {
	totalRevenue: number;
	totalCollected: number;
	totalRefunded: number;
	totalPayments: number;
	refundedPayments: number;
	uniquePayers: number;
	periodRevenue: number;
	periodPayments: number;
	revenueChangePercent: number | null;
	paymentsChangePercent: number | null;
	dailyStats: IFlowPaymentDay[];
	recentPayments: IFlowPaymentItem[];
}

export function flowPaymentsPath(
	appId: string,
	startDate?: string,
	endDate?: string,
	limit?: number,
): string {
	const params = new URLSearchParams();
	if (startDate) params.set("start_date", startDate);
	if (endDate) params.set("end_date", endDate);
	if (limit !== undefined) params.set("limit", limit.toString());
	const query = params.toString();
	const path = `apps/${encodeURIComponent(appId)}/sales/flow-payments`;
	return query ? `${path}?${query}` : path;
}

export interface IDiscount {
	id: string;
	appId: string;
	code: string;
	name: string;
	description: string | null;
	discountType: "Percentage" | "FixedAmount";
	discountValue: number;
	maxUses: number | null;
	usedCount: number;
	minPurchaseAmount: number | null;
	startsAt: string;
	expiresAt: string | null;
	isActive: boolean;
	isValid: boolean;
	createdAt: string;
}

export interface ICreateDiscountRequest {
	code: string;
	name: string;
	description?: string;
	discountType: "percentage" | "fixed_amount";
	discountValue: number;
	maxUses?: number;
	minPurchaseAmount?: number;
	startsAt?: string;
	expiresAt?: string;
}

export interface IUpdateDiscountRequest {
	code?: string;
	name?: string;
	description?: string;
	discountType?: "percentage" | "fixed_amount";
	discountValue?: number;
	maxUses?: number;
	minPurchaseAmount?: number;
	startsAt?: string;
	expiresAt?: string;
	isActive?: boolean;
}

export interface ISalesState {
	/**
	 * Get sales overview for an app
	 */
	getSalesOverview(appId: string): Promise<ISalesOverview>;

	/**
	 * Get detailed sales stats with daily breakdown
	 */
	getSalesStats(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
	): Promise<ISalesStats>;

	/**
	 * Get the revenue the app's flows collected, with a daily breakdown for
	 * the range and the most recent payments
	 */
	getFlowPayments(
		appId: string,
		startDate?: string,
		endDate?: string,
		limit?: number,
	): Promise<IFlowPaymentsReport>;

	/**
	 * List purchases for an app
	 */
	listPurchases(
		appId: string,
		status?: string,
		offset?: number,
		limit?: number,
	): Promise<IPurchasesResponse>;

	/**
	 * Update the app price
	 */
	updatePrice(
		appId: string,
		price: number,
	): Promise<{ price: number; updated: boolean }>;

	/**
	 * List all discounts for an app
	 */
	listDiscounts(appId: string, activeOnly?: boolean): Promise<IDiscount[]>;

	/**
	 * Get a specific discount
	 */
	getDiscount(appId: string, discountId: string): Promise<IDiscount>;

	/**
	 * Create a new discount
	 */
	createDiscount(
		appId: string,
		discount: ICreateDiscountRequest,
	): Promise<IDiscount>;

	/**
	 * Update a discount
	 */
	updateDiscount(
		appId: string,
		discountId: string,
		updates: IUpdateDiscountRequest,
	): Promise<IDiscount>;

	/**
	 * Delete a discount
	 */
	deleteDiscount(appId: string, discountId: string): Promise<void>;

	/**
	 * Toggle discount active state
	 */
	toggleDiscount(appId: string, discountId: string): Promise<IDiscount>;
}
