"use client";

import { UsePageContent } from "@tm9657/flow-like-ui";
import { EVENT_CONFIG } from "../../lib/event-config";
import NotFound from "../library/config/not-found";

export default function UsePage() {
	return <UsePageContent eventConfig={EVENT_CONFIG} notFound={<NotFound />} />;
}
