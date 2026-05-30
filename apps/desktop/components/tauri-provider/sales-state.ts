import type {
	ICreateDiscountRequest,
	IDiscount,
	IPurchasesResponse,
	ISalesOverview,
	ISalesState,
	ISalesStats,
	IUpdateDiscountRequest,
} from "@flow-like/flow-like-ui";
import { fetcher, post } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class SalesState implements ISalesState {
	constructor(private readonly backend: TauriBackend) {}

	async getSalesOverview(appId: string): Promise<ISalesOverview> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		return await fetcher<ISalesOverview>(
			this.backend.profile,
			`apps/${appId}/sales`,
			undefined,
			this.backend.auth,
		);
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
		return await fetcher<ISalesStats>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
		);
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
		return await fetcher<IPurchasesResponse>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
		);
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
		return await fetcher<IDiscount[]>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
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
