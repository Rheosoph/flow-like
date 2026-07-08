"use client";

import { UsePageContent } from "@flow-like/flow-like-ui";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";
import NotFound from "../library/config/not-found";

export default function UsePage() {
	return <UsePageContent eventConfig={EVENT_CONFIG} notFound={<NotFound />} />;
}
