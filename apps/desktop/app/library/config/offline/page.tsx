"use client";

import { OfflineAccessPage } from "@flow-like/flow-like-ui/components/settings/offline-access";
import { useSearchParams } from "next/navigation";
import type React from "react";
import NotFound from "../not-found";

export default function Page(): React.ReactElement {
	const searchParams = useSearchParams();
	const id = searchParams?.get("id") ?? null;

	if (!id) return <NotFound />;

	return <OfflineAccessPage appId={id} />;
}
