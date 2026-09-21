export interface PaymentLegalDocument {
	kind: "PAYMENTS_OWNER_TERMS" | "SELLER_TERMS" | "PURCHASE_TERMS";
	version: string;
	locale: string;
	text: string;
}

const slugs: Record<PaymentLegalDocument["kind"], string> = {
	PAYMENTS_OWNER_TERMS: "owner-terms",
	SELLER_TERMS: "seller-terms",
	PURCHASE_TERMS: "purchase-terms",
};

const versions = import.meta.glob<PaymentLegalDocument[]>(
	"../content/legal/payments/*.json",
	{ eager: true, import: "default" },
);

export const paymentLegalDocuments = Object.values(versions).flat();

export function paymentLegalPath(document: PaymentLegalDocument): string {
	return `/legal/payments/${slugs[document.kind]}/${document.version}/${document.locale}/`;
}

export function paymentLegalPaths() {
	return paymentLegalDocuments.map((document) => ({
		params: {
			slug: slugs[document.kind],
			version: document.version,
			locale: document.locale,
		},
		props: { document },
	}));
}
