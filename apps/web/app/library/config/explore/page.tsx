"use client";

import { ExploreDataPage } from "@tm9657/flow-like-ui/components/settings/explore/explore-page";
import { useSearchParams } from "next/navigation";
import type React from "react";
import NotFound from "../not-found";

export default function Page(): React.ReactElement {
	const searchParams = useSearchParams();
	const id = searchParams?.get("id") ?? null;

	if (!id) return <NotFound />;

	return <ExploreDataPage appId={id} />;
}