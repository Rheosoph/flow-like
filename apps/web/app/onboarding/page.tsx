"use client";

import { useGuideStore } from "@flow-like/flow-like-ui";
import { useRouter } from "next/navigation";
import { useEffect } from "react";

/** Onboarding now happens in the app itself: Home asks the welcome question. */
export default function Onboarding() {
	const router = useRouter();
	const requestWelcome = useGuideStore((state) => state.requestWelcome);

	useEffect(() => {
		requestWelcome();
		router.replace("/");
	}, [requestWelcome, router]);

	return null;
}
