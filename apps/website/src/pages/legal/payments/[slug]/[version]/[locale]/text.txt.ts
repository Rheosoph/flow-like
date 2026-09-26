import type { APIRoute } from "astro";
import {
	type PaymentLegalDocument,
	paymentLegalPaths,
} from "../../../../../../lib/payment-legal";

export const prerender = true;
export const getStaticPaths = paymentLegalPaths;

export const GET: APIRoute = ({ props }) => {
	const document = props.document as PaymentLegalDocument;
	// Consent hashes use these exact UTF-8 bytes, including whitespace.
	return new Response(document.text, {
		headers: { "Content-Type": "text/plain; charset=utf-8" },
	});
};
