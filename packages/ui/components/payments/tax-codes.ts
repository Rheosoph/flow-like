"use client";

import { useQuery } from "@tanstack/react-query";

export type TaxCodeGroup =
	| "common"
	| "digital"
	| "services"
	| "events"
	| "physical";

export interface StripeTaxCode {
	readonly id: string;
	readonly name: string;
	readonly description: string;
	readonly group: TaxCodeGroup;
}

export const TAX_CODE_GROUPS: readonly TaxCodeGroup[] = [
	"common",
	"digital",
	"services",
	"events",
	"physical",
];

export function isStripeTaxCode(value: string): boolean {
	return /^txcd_\d{8}$/.test(value);
}

/** Stripe's published list, kept out of the main bundle because it is ~220 KB. */
export function useStripeTaxCodes() {
	return useQuery({
		queryKey: ["stripe-tax-codes"],
		queryFn: async () => {
			const { default: list } = await import("./stripe-tax-codes.json");
			return list.codes as readonly StripeTaxCode[];
		},
		staleTime: Number.POSITIVE_INFINITY,
		gcTime: Number.POSITIVE_INFINITY,
		meta: { persist: false },
	});
}
