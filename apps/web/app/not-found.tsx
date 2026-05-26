"use client";

import { NotFoundPage } from "@flow-like/flow-like-ui";
import { useRouter } from "next/navigation";

export default function NotFound() {
	const router = useRouter();

	return (
		<NotFoundPage
			onGoBack={() => router.back()}
			onGoHome={() => router.push("/")}
		/>
	);
}
