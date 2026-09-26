import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";

export interface PaymentPromptRef {
	id: string;
	appId: string;
	runId: string;
}
export interface PaymentSimulation {
	id: string;
	status: string;
	amountMinor: number;
	currency: string;
	productName: string;
}
export const PAYMENT_SIMULATION_EVENT = "flow-like:payment-simulation";
export const PAYMENT_PROMPT_EVENT = "flow-like:payment-request";

export function paymentPromptRef(value: unknown): PaymentPromptRef | null {
	if (!value || typeof value !== "object") return null;
	const object = value as Partial<PaymentPromptRef>;
	const opaque = (item: unknown): item is string =>
		typeof item === "string" && /^[a-zA-Z0-9_-]{1,128}$/.test(item);
	if (!opaque(object.id) || !opaque(object.appId) || !opaque(object.runId))
		return null;
	return { id: object.id, appId: object.appId, runId: object.runId };
}

export function dispatchPaymentRequest(event: IIntercomEvent): void {
	if (typeof window === "undefined") return;
	if (event.event_type === "payment_simulation") {
		const value = event.payload;
		if (
			value &&
			typeof value.id === "string" &&
			value.id.startsWith("sim_") &&
			typeof value.productName === "string" &&
			typeof value.status === "string" &&
			Number.isSafeInteger(value.amountMinor) &&
			value.currency === "eur"
		) {
			window.dispatchEvent(
				new CustomEvent(PAYMENT_SIMULATION_EVENT, { detail: value }),
			);
		}
		return;
	}
	if (event.event_type !== "payment_request") return;
	const detail = paymentPromptRef(event.payload);
	if (detail)
		window.dispatchEvent(new CustomEvent(PAYMENT_PROMPT_EVENT, { detail }));
}
