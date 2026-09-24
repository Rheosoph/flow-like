import {
	type ICreateDiscountRequest,
	type IDiscount,
	type IFlowPaymentsReport,
	type IPurchasesResponse,
	type ISalesOverview,
	type ISalesState,
	type ISalesStats,
	type IUpdateDiscountRequest,
	flowPaymentsPath,
} from "@flow-like/flow-like-ui";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { fetcher, post } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class SalesState implements ISalesState {
	constructor(private readonly backend: TauriBackend) {}

	async getSalesOverview(appId: string): Promise<ISalesOverview> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		const overview = await fetcher<ISalesOverview>(
			this.backend.profile,
			`apps/${appId}/sales`,
			undefined,
			this.backend.auth,
		);
		if (!isRecord(overview)) {
			throw new Error(`Unexpected sales overview response for app ${appId}`);
		}
		return overview;
	}

	async getSalesStats(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
	): Promise<ISalesStats> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		const params = new URLSearchParams();
		if (startDate) params.set("start_date", startDate);
		if (endDate) params.set("end_date", endDate);
		if (period) params.set("period", period);

		const query = params.toString();
		const url = query
			? `apps/${appId}/sales/stats?${query}`
			: `apps/${appId}/sales/stats`;
		const stats = await fetcher<ISalesStats>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
		);
		if (!isRecord(stats)) {
			throw new Error(`Unexpected sales stats response for app ${appId}`);
		}
		return { ...stats, dailyStats: asArray(stats.dailyStats) };
	}

	async getFlowPayments(
		appId: string,
		startDate?: string,
		endDate?: string,
		limit?: number,
	): Promise<IFlowPaymentsReport> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		const report = await fetcher<IFlowPaymentsReport>(
			this.backend.profile,
			flowPaymentsPath(appId, startDate, endDate, limit),
			undefined,
			this.backend.auth,
		);
		if (!isRecord(report)) {
			throw new Error(`Unexpected flow payments response for app ${appId}`);
		}
		return {
			...report,
			dailyStats: asArray(report.dailyStats),
			recentPayments: asArray(report.recentPayments),
		};
	}

	async listPurchases(
		appId: string,
		status?: string,
		offset?: number,
		limit?: number,
	): Promise<IPurchasesResponse> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		const params = new URLSearchParams();
		if (status) params.set("status", status);
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		const query = params.toString();
		const url = query
			? `apps/${appId}/sales/purchases?${query}`
			: `apps/${appId}/sales/purchases`;
		const response = await fetcher<IPurchasesResponse>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
		);
		if (!isRecord(response)) {
			throw new Error(`Unexpected purchases response for app ${appId}`);
		}
		return { ...response, purchases: asArray(response.purchases) };
	}

	async updatePrice(
		appId: string,
		price: number,
	): Promise<{ price: number; updated: boolean }> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		return await fetcher<{ price: number; updated: boolean }>(
			this.backend.profile,
			`apps/${appId}/sales/price`,
			{
				method: "PATCH",
				body: JSON.stringify({ price }),
			},
			this.backend.auth,
		);
	}

	async listDiscounts(
		appId: string,
		activeOnly?: boolean,
	): Promise<IDiscount[]> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		const params = new URLSearchParams();
		if (activeOnly) params.set("active_only", "true");

		const query = params.toString();
		const url = query
			? `apps/${appId}/sales/discounts?${query}`
			: `apps/${appId}/sales/discounts`;
		return asArray(
			await fetcher<IDiscount[]>(
				this.backend.profile,
				url,
				undefined,
				this.backend.auth,
			),
		);
	}

	async getDiscount(appId: string, discountId: string): Promise<IDiscount> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		return await fetcher<IDiscount>(
			this.backend.profile,
			`apps/${appId}/sales/discounts/${discountId}`,
			undefined,
			this.backend.auth,
		);
	}

	async createDiscount(
		appId: string,
		discount: ICreateDiscountRequest,
	): Promise<IDiscount> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		return await post<IDiscount>(
			this.backend.profile,
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
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		return await fetcher<IDiscount>(
			this.backend.profile,
			`apps/${appId}/sales/discounts/${discountId}`,
			{
				method: "PATCH",
				body: JSON.stringify(updates),
			},
			this.backend.auth,
		);
	}

	async deleteDiscount(appId: string, discountId: string): Promise<void> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		await fetcher<void>(
			this.backend.profile,
			`apps/${appId}/sales/discounts/${discountId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}

	async toggleDiscount(appId: string, discountId: string): Promise<IDiscount> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		return await post<IDiscount>(
			this.backend.profile,
			`apps/${appId}/sales/discounts/${discountId}/toggle`,
			{},
			this.backend.auth,
		);
	}
}
