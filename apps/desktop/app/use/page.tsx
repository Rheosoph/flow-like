"use client";

import { UsePageContent } from "@flow-like/flow-like-ui/components/interfaces/use-page-content";
// Imported by module path rather than through the package barrel: the barrel re-exports the
// whole component library, which would put the flow editor and every chart library in this
// route's first load.
import { USE_EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config-use";
import NotFound from "../library/config/not-found";

export default function UsePage() {
	return (
		<UsePageContent eventConfig={USE_EVENT_CONFIG} notFound={<NotFound />} />
	);
}
