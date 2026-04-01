"use client";

import { AppPackagesPage } from "@tm9657/flow-like-ui";
import { useSearchParams } from "next/navigation";

export default function Page() {
	const searchParams = useSearchParams();
	const id = searchParams.get("id");

	if (!id) return null;

	return <AppPackagesPage appId={id} />;
}
