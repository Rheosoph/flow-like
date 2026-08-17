"use client";

import { useTranslation } from "@flow-like/locales";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import type { IMetadata } from "../../types";

export interface PageRouteManagerProps {
	appId: string;
	pages: Array<[string, string | null, string | null, IMetadata | null]>;
	routes: IRouteMapping[];
	onDeletePage: (pageId: string, boardId: string | null) => Promise<void>;
	onCreatePage: (name: string, boardId?: string) => Promise<void>;
	onSetRoute: (path: string, eventId: string) => Promise<void>;
	onDeleteRoute: (path: string) => Promise<void>;
	onNavigate: (path: string) => void;
}

/**
 * @deprecated This component uses the old route model and is not used.
 * Routes are now managed at the app level via events page.
 */
export function PageRouteManager(_props: PageRouteManagerProps) {
	const { t } = useTranslation("common");
	return (
		<div className="p-4 text-muted-foreground">
			<p>{t('routeManagementHasBeenMovedToTheEventsPage', 'Route management has been moved to the Events page.')}</p>
		</div>
	);
}
