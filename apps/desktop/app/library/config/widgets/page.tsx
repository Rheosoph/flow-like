"use client";

import { WidgetDetail, WidgetList } from "@flow-like/flow-like-ui";
import { useSearchParams } from "next/navigation";

export default function WidgetsPage() {
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const widgetId = searchParams.get("widgetId");

	if (widgetId) {
		return <WidgetDetail appId={appId} widgetId={widgetId} />;
	}

	return <WidgetList appId={appId} />;
}
