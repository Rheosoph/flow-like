"use client";

import { UseRoutePage } from "@flow-like/flow-like-ui/components/interfaces/use-route-page";
import { USE_EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config-use";
import NotFound from "../library/config/not-found";

export default function UsePage() {
	return (
		<UseRoutePage eventConfig={USE_EVENT_CONFIG} notFound={<NotFound />} />
	);
}
