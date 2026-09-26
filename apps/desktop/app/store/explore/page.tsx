"use client";

import { ExplorePage } from "@flow-like/flow-like-ui";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";

export default function Page() {
	return <ExplorePage eventConfig={EVENT_CONFIG} />;
}
