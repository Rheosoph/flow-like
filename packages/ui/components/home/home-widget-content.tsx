"use client";

import {
	HomeAiUsage,
	HomeExecutionsByApp,
	HomeNeedsAttention,
	HomeNotifications,
	HomeRecentRuns,
	HomeRunActivity,
	HomeRunStats,
	HomeSchedules,
} from "./home-content/activity";
import { HomeAppEmbed } from "./home-content/app-embed";
import {
	HomeAppCollection,
	HomeCategories,
	HomeModels,
	HomePackages,
} from "./home-content/collections";
import type { HomeContentProps } from "./home-content/config";
import {
	HomeFlowPilot,
	HomeGreeting,
	HomeInformation,
	HomeQuickActions,
	HomeQuickLinks,
} from "./home-content/personal-content";
import { HomeEmpty } from "./home-content/shared";

import {
	HomeAppCollectionFeature,
	HomeAppRanking,
	HomeAppSpotlight,
	HomeModelSpotlight,
} from "./home-content/discovery";
import { HomeSectionHeading } from "./home-content/section-heading";
import { HomeWorkspacePulse } from "./home-content/workspace-overview";

export function HomeWidgetContent(props: HomeContentProps) {
	switch (props.widget.type) {
		case "section-heading":
			return <HomeSectionHeading {...props} />;
		case "app-spotlight":
			return <HomeAppSpotlight {...props} />;
		case "app-ranking":
			return <HomeAppRanking {...props} />;
		case "app-collection-feature":
			return <HomeAppCollectionFeature {...props} />;
		case "model-spotlight":
			return <HomeModelSpotlight {...props} />;
		case "workspace-pulse":
			return <HomeWorkspacePulse {...props} />;
		case "flowpilot":
			return <HomeFlowPilot {...props} />;
		case "app-embed":
			return <HomeAppEmbed {...props} />;
		case "app-collection":
			return <HomeAppCollection {...props} />;
		case "categories":
			return <HomeCategories {...props} />;
		case "packages":
			return <HomePackages {...props} />;
		case "models":
			return <HomeModels {...props} />;
		case "quick-links":
			return <HomeQuickLinks {...props} />;
		case "quick-actions":
			return <HomeQuickActions {...props} />;
		case "greeting":
			return <HomeGreeting {...props} />;
		case "information":
			return <HomeInformation {...props} />;
		case "notifications":
			return <HomeNotifications {...props} />;
		case "needs-attention":
			return <HomeNeedsAttention {...props} />;
		case "run-activity":
			return <HomeRunActivity {...props} />;
		case "executions-by-app":
			return <HomeExecutionsByApp {...props} />;
		case "ai-usage":
			return <HomeAiUsage {...props} />;
		case "run-stats":
			return <HomeRunStats {...props} />;
		case "recent-runs":
			return <HomeRecentRuns {...props} />;
		case "schedules":
			return <HomeSchedules {...props} />;
		default:
			return (
				<HomeEmpty>This widget is not available in this version.</HomeEmpty>
			);
	}
}
