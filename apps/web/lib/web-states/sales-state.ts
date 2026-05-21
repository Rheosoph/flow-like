import type {
	ICreateDiscountRequest,
	IDiscount,
	IPurchasesResponse,
	ISalesOverview,
	ISalesState,
	ISalesStats,
	IUpdateDiscountRequest,
} from "@flow-like/flow-like-ui";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
} from "./api-utils";

export class WebSalesState implements ISalesState {
	constructor(private readonly backend: WebBackendRef) {}

	async getSalesOverview(appId: string): Promise<ISalesOverview> {
		return await apiGet<ISalesOverview>(
			`apps/${appId}/sales`,
			this.backend.auth,
		);
	}

	async getSalesStats(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
	): Promise<ISalesStats> {
		const params = new URLSearchParams();
		if (startDate) params.set("start_date", startDate);
		if (endDate) params.set("end_date", endDate);
		if (period) params.set("period", period);

		const query = params.toString();
		const url = query
			? `apps/${appId}/sales/stats?${query}`
			: `apps/${appId}/sales/stats`;
		return await apiGet<ISalesStats>(url, this.backend.auth);
	}

	async listPurchases(
		appId: string,
		status?: string,
		offset?: number,
		limit?: number,
	): Promise<IPurchasesResponse> {
		const params = new URLSearchParams();
		if (status) params.set("status", status);
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		const query = params.toString();
		const url = query
			? `apps/${appId}/sales/purchases?${query}`
			: `apps/${appId}/sales/purchases`;
		return await apiGet<IPurchasesResponse>(url, this.backend.auth);
	}

	async updatePrice(
		appId: string,
		price: number,
	): Promise<{ price: number; updated: boolean }> {
		return await apiPatch<{ price: number; updated: boolean }>(
			`apps/${appId}/sales/price`,
			{ price },
			this.backend.auth,
		);
	}

	async listDiscounts(
		appId: string,
		activeOnly?: boolean,
	): Promise<IDiscount[]> {
		const params = new URLSearchParams();
		if (activeOnly) params.set("active_only", "true");

		const query = params.toString();
		const url = query
			? `apps/${appId}/sales/discounts?${query}`
			: `apps/${appId}/sales/discounts`;
		return await apiGet<IDiscount[]>(url, this.backend.auth);
	}

	async getDiscount(appId: string, discountId: string): Promise<IDiscount> {
		return await apiGet<IDiscount>(
			`apps/${appId}/sales/discounts/${discountId}`,
			this.backend.auth,
		);
	}

	async createDiscount(
		appId: string,
		discount: ICreateDiscountRequest,
	): Promise<IDiscount> {
		return await apiPost<IDiscount>(
			`apps/${appId}/sales/discounts`,
			discount,
			this.backend.auth,
		);
	}

	async updateDiscount(
		appId: string,
		discountId: string,
		updates: IUpdateDiscountRequest,
	): Promise<IDiscount> {
		return await apiPatch<IDiscount>(
			`apps/${appId}/sales/discounts/${discountId}`,
			updates,
			this.backend.auth,
		);
	}

	async deleteDiscount(appId: string, discountId: string): Promise<void> {
		await apiDelete(
			`apps/${appId}/sales/discounts/${discountId}`,
			this.backend.auth,
		);
	}

	async toggleDiscount(appId: string, discountId: string): Promise<IDiscount> {
		return await apiPost<IDiscount>(
			`apps/${appId}/sales/discounts/${discountId}/toggle`,
			{},
			this.backend.auth,
		);
	}
}
