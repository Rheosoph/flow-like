"use client";

import { DataStudioPage } from "@flow-like/flow-like-ui/components/settings/explore/explore-page";
import { useSearchParams } from "next/navigation";
import type React from "react";
import NotFound from "../not-found";

export default function Page(): React.ReactElement {
	const searchParams = useSearchParams();
	const id = searchParams?.get("id") ?? null;

	if (!id) return <NotFound />;

	return <DataStudioPage appId={id} />;
}
