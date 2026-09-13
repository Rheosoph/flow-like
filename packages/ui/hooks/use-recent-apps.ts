"use client";

import { useEffect, useState } from "react";
import {
	readRecentApps,
	RECENT_APPS_CHANGED,
	type RecentAppUse,
} from "../lib/recent-apps";

export function useRecentApps(scope: readonly string[]) {
	const signature = JSON.stringify(scope);
	const [snapshot, setSnapshot] = useState<{
		scope: string;
		rows: RecentAppUse[];
	}>({ scope: "", rows: [] });
	useEffect(() => {
		const refresh = () =>
			setSnapshot({
				scope: signature,
				rows: readRecentApps(JSON.parse(signature)),
			});
		refresh();
		window.addEventListener(RECENT_APPS_CHANGED, refresh);
		window.addEventListener("storage", refresh);
		return () => {
			window.removeEventListener(RECENT_APPS_CHANGED, refresh);
			window.removeEventListener("storage", refresh);
		};
	}, [signature]);
	return snapshot.scope === signature ? snapshot.rows : [];
}
