export interface PaymentTerms {
	kind: string;
	version: string;
	locale: string;
	text: string;
	hash: string;
	url?: string | null;
}

export interface ConnectAccount {
	platformOwned: boolean;
	state: string;
	canAcceptPayments: boolean;
	canSell: boolean;
	payoutsEnabled: boolean;
	country?: string | null;
	defaultCurrency?: string | null;
	business?: { name?: string; url?: string; support_email?: string } | null;
	requirements?: {
		currently_due?: string[];
		past_due?: string[];
		disabled_reason?: string | null;
	} | null;
	dashboardUrl?: string | null;
}

export interface AppPaymentSettings {
	paymentsEnabled: boolean;
	refundsFromFlows: boolean;
	maxPaymentAmount: number;
	refundsDailyCap: number;
	currency: string;
	platformMaxPaymentAmount: number;
	adminBlocked: boolean;
}

export interface PaymentReadiness {
	platformOwned: boolean;
	payee: { isYou: boolean };
	paymentsEnabled: boolean;
	canAcceptPayments: boolean;
	canSell: boolean;
	state: string;
}

export interface PurchaseOrder {
	platformOwned?: boolean;
	orderId: string;
	appId: string;
	/** Absent on servers that only sold apps through the marketplace. */
	itemKind?: "APP" | "PACKAGE";
	itemId?: string;
	itemName?: string;
	status: string;
	amount: number;
	currency: string;
	checkoutUrl?: string | null;
	receiptUrl?: string | null;
	refundedAmount: number;
	pendingRefundAmount: number;
	withdrawable: boolean;
	refundableRemaining?: number;
	withdrawDeadline?: number;
}

export interface WithdrawalRequest {
	confirm: true;
	consumerName: string;
	confirmationEmail: string;
}

type OrderItem = Pick<PurchaseOrder, "appId" | "itemKind" | "itemId">;

/** Store page of what the order bought: package orders link to the package registry. */
export function orderItemHref(order: OrderItem): string {
	return order.itemKind === "PACKAGE"
		? `/store/packages?id=${encodeURIComponent(order.itemId ?? order.appId)}`
		: `/store?id=${encodeURIComponent(order.appId)}`;
}

export function orderItemName(
	order: OrderItem & Pick<PurchaseOrder, "itemName">,
): string {
	return order.itemName ?? order.itemId ?? order.appId;
}

export const pendingOrder = (status: string) =>
	[
		"CREATED",
		"CANCEL_PENDING",
		"REFUND_REQUIRED",
		"CREATING",
		"OPEN",
		"OPENING",
		"AWAITING_PAYMENT",
		"PROCESSING",
		"CANCEL_REQUESTED",
		"REFUND_PENDING",
	].includes(status);

/** EUR input stays decimal text until the integer minor-unit conversion. */
export function parseEuroAmount(input: string): number | null {
	const match = /^(0|[1-9]\d*)(?:[.,](\d{1,2}))?$/.exec(input.trim());
	if (!match) return null;
	const minor =
		BigInt(match[1]) * 100n + BigInt((match[2] ?? "").padEnd(2, "0"));
	return minor <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(minor) : null;
}

export function amountInput(minor: number): string {
	if (!Number.isSafeInteger(minor) || minor < 0) return "";
	return `${Math.floor(minor / 100)}.${String(minor % 100).padStart(2, "0")}`;
}

export function paymentMoney(
	minor: number,
	currency: string,
	locale?: string,
): string {
	if (!Number.isSafeInteger(minor)) return "";
	return new Intl.NumberFormat(locale, { style: "currency", currency }).format(
		minor / 100,
	);
}

export function paymentUrl(value?: string | null): string | undefined {
	if (!value) return undefined;
	try {
		const url = new URL(value);
		return url.protocol === "https:" && !url.username && !url.password
			? url.href
			: undefined;
	} catch {
		return undefined;
	}
}
