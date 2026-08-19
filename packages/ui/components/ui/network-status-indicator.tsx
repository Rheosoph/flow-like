"use client";

import { useTranslation } from "@flow-like/locales";
import { WifiIcon, WifiOffIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useNetworkStatus } from "../../hooks/use-network-status";
import { cn } from "../../lib/utils";

export function NetworkStatusIndicator() {
	const { t } = useTranslation("common");
	const isOnline = useNetworkStatus();
	const [showOfflineMessage, setShowOfflineMessage] = useState(false);

	useEffect(() => {
		if (!isOnline) {
			setShowOfflineMessage(true);
		} else {
			// Keep message visible for a bit when coming back online
			const timer = setTimeout(() => setShowOfflineMessage(false), 3000);
			return () => clearTimeout(timer);
		}
	}, [isOnline]);

	// Don't show anything if online and message timer has expired
	if (!showOfflineMessage) {
		return null;
	}

	return (
		<div
			className={cn(
				"fixed bottom-4 right-4 z-50 flex items-center gap-2 rounded-lg px-4 py-2 text-sm font-medium shadow-lg transition-all duration-300",
				isOnline
					? "bg-green-500 text-white"
					: "bg-orange-500 text-white animate-pulse",
			)}
		>
			{isOnline ? (
				<>
					<WifiIcon className="h-4 w-4" />
					<span>{t('backOnline', 'Back Online')}</span>
				</>
			) : (
				<>
					<WifiOffIcon className="h-4 w-4" />
					<span>{t('offlineUsingCachedData', 'Offline - Using Cached Data')}</span>
				</>
			)}
		</div>
	);
}
