"use client";

import { BOTS_ROUTE } from "@flow-like/flow-like-ui/components/bots-tab/dock-selection";
import { useRouter, useSearchParams } from "next/navigation";
import { useEffect } from "react";

/**
 * The Bots tab moved from `/chat` to `/bots`. A static export has no server redirects, so this
 * forwards on the client, keeping `?bot=`, `?conversation=` and anything else the link carried.
 */
export default function Page() {
	const router = useRouter();
	const searchParams = useSearchParams();

	useEffect(() => {
		const query = searchParams.toString();
		router.replace(`${BOTS_ROUTE}${query ? `?${query}` : ""}`);
	}, [router, searchParams]);

	return null;
}
