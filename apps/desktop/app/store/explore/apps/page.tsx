"use client";

import { ExploreAppsPage } from "@flow-like/flow-like-ui";
import { EVENT_CONFIG } from "../../../../lib/event-config";

export default function Page() {
	return <ExploreAppsPage eventConfig={EVENT_CONFIG} />;
}
