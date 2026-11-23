"use client";

import { guideWelcomeHref } from "@flow-like/flow-like-ui";
import { useRouter } from "next/navigation";
import { useEffect } from "react";

export default function Onboarding() {
	const router = useRouter();

	useEffect(() => {
		router.replace(guideWelcomeHref());
	}, [router]);

	return null;
}
