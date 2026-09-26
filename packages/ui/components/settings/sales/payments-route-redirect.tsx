"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { useEffect } from "react";

/** Payment settings moved into the Monetization section; keep old links working. */
export function PaymentsRouteRedirect() {
	const router = useRouter();
	const id = useSearchParams().get("id");

	useEffect(() => {
		const params = new URLSearchParams({ tab: "settings" });
		if (id) params.set("id", id);
		router.replace(`/library/config/sales?${params}`);
	}, [id, router]);

	return null;
}
