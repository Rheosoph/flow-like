"use client";

import { ExploreBrowsePage } from "@flow-like/flow-like-ui";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";

export default function Page() {
	return <ExploreBrowsePage eventConfig={EVENT_CONFIG} canScaffold />;
}
