"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Card,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@flow-like/flow-like-ui";
import { MonitorIcon } from "lucide-react";

export default function StatisticsPage() {
	const { t } = useTranslation("common");
	return (
		<div className="h-full flex flex-col max-h-full overflow-auto min-h-0">
			<div className="container mx-auto px-2 pb-4 flex flex-col gap-6">
				<div className="flex flex-col gap-1 pt-2">
					<h1 className="text-2xl font-bold">{t('boardStatistics', 'Board Statistics')}</h1>
					<p className="text-muted-foreground">
						{t('nodeUsageCategoryDistributionAndBoardAnalytics', 'Node usage, category distribution, and board analytics')}
					</p>
				</div>

				<Card>
					<CardHeader className="text-center">
						<div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-muted">
							<MonitorIcon className="h-6 w-6 text-muted-foreground" />
						</div>
						<CardTitle className="text-lg">{t('desktopFeature', 'Desktop Feature')}</CardTitle>
						<CardDescription>
							{t('boardStatisticsRequiresTheDesktopAppToScanYourLocalBoardsForNodeUsagePatternsAndAnalyticsThisFeatureIsNotYetAvailableInTheWebVersion', "Board Statistics requires the desktop app to scan your local boards for node usage, patterns, and analytics. This feature is not yet available in the web version.")}
						</CardDescription>
					</CardHeader>
				</Card>
			</div>
		</div>
	);
}
