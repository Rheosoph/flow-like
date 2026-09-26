"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { IAppType } from "../../../lib/schema/app/app";
import type { WasmPackageCategory } from "../../../lib/schema/wasm";
import type {
	ExplorePermissionFacet,
	ExploreRailKey,
	ExploreSearchSort,
	ExploreSearchType,
	ExploreTone,
	ExploreTypeFilter,
} from "./explore-types";

type StoreT = ReturnType<typeof useTranslation>["t"];

export function useExploreLabels() {
	const { t } = useTranslation("store");
	return useMemo(
		() => ({
			tone: (tone: ExploreTone) => toneLabel(t, tone),
			railTitle: (rail: ExploreRailKey) => railTitle(t, rail),
			railSubtitle: (rail: ExploreRailKey, mix: ExploreTypeFilter) =>
				railSubtitle(t, rail, mix),
			sort: (sort: ExploreSearchSort) => sortLabel(t, sort),
			permission: (permission: ExplorePermissionFacet) =>
				permissionLabel(t, permission),
			type: (type: ExploreSearchType) => typeLabel(t, type),
			appType: (type?: IAppType | null) => appTypeLabel(t, type),
			packageCategory: (category: WasmPackageCategory) =>
				packageCategoryLabel(t, category),
			kindCounts: (apps: number, packages: number) =>
				kindCounts(t, apps, packages),
		}),
		[t],
	);
}

export function formatPrice(cents: number): string {
	return `€${(cents / 100).toFixed(2)}`;
}

function toneLabel(t: StoreT, tone: ExploreTone): string {
	switch (tone) {
		case "info":
			return t("exploreToneInfo", "Info");
		case "launch":
			return t("exploreToneLaunch", "Launch");
		case "maintenance":
			return t("exploreToneMaintenance", "Maintenance");
		case "warning":
			return t("exploreToneWarning", "Warning");
	}
}

function railTitle(t: StoreT, rail: ExploreRailKey): string {
	switch (rail) {
		case "trending":
			return t("popularRightNow", "Popular right now");
		case "new":
		case "new_count":
			return t("exploreRailNew", "New this week");
		case "top_paid":
			return t("exploreRailTopPaid", "Top paid");
		case "for_builders":
			return t("exploreRailForBuilders", "For builders");
		case "by_category":
			return t("browseCategories", "Browse categories");
		case "suites":
			return t("suitesAmpPlatforms", "Suites & Platforms");
	}
}

function railSubtitle(
	t: StoreT,
	rail: ExploreRailKey,
	mix: ExploreTypeFilter,
): string | undefined {
	switch (rail) {
		case "trending":
			if (mix === "apps") {
				return t("exploreTrendingSubtitleApps", "Most installed apps");
			}
			if (mix === "packages") {
				return t("exploreTrendingSubtitlePackages", "Most installed packages");
			}
			return t(
				"exploreTrendingSubtitleAll",
				"Most installed apps and packages",
			);
		case "new":
			if (mix === "apps") {
				return t("exploreNewSubtitleApps", "Fresh apps from the community");
			}
			if (mix === "packages") {
				return t(
					"exploreNewSubtitlePackages",
					"Fresh packages from the community",
				);
			}
			return t("exploreNewSubtitleAll", "Fresh from the community");
		case "top_paid":
			return t("exploreTopPaidSubtitle", "Best sellers this month");
		case "for_builders":
			return t(
				"exploreForBuildersSubtitle",
				"Node packages that add new nodes to your flows",
			);
		default:
			return undefined;
	}
}

function sortLabel(t: StoreT, sort: ExploreSearchSort): string {
	switch (sort) {
		case "best":
			return t("exploreSortBest", "Best match");
		case "newest":
			return t("newest", "Newest");
		case "rating":
			return t("bestRated", "Best rated");
		case "installs":
			return t("exploreSortInstalls", "Most installs");
		case "name":
			return t("exploreSortName", "Name");
		case "updated":
			return t("exploreSortUpdated", "Recently updated");
	}
}

function permissionLabel(
	t: StoreT,
	permission: ExplorePermissionFacet,
): string {
	switch (permission) {
		case "none":
			return t("explorePermissionNone", "None requested");
		case "network":
			return t("explorePermissionNetwork", "Network");
		case "models":
			return t("explorePermissionModels", "Models");
		case "storage":
			return t("explorePermissionStorage", "Storage");
	}
}

function typeLabel(t: StoreT, type: ExploreSearchType): string {
	switch (type) {
		case "all":
			return t("exploreTypeAll", "All");
		case "apps":
			return t("apps", "Apps");
		case "packages":
			return t("packages", "Packages");
		case "collections":
			return t("exploreTypeCollections", "Collections");
	}
}

/** `undefined` for an app its owner has not classified, so the label is left out rather than shown as a gap. */
function appTypeLabel(
	t: StoreT,
	type: IAppType | null | undefined,
): string | undefined {
	switch (type) {
		case IAppType.Agent:
			return t("exploreAppTypeAgent", "Agent");
		case IAppType.CustomInterface:
			return t("exploreAppTypeCustomInterface", "Custom Interface");
		case IAppType.DataFocus:
			return t("exploreAppTypeDataFocus", "Data Focus");
		case IAppType.DataPipeline:
			return t("exploreAppTypeDataPipeline", "Data Pipeline");
		case IAppType.Analytics:
			return t("exploreAppTypeAnalytics", "Dashboards & Analytics");
		case IAppType.Form:
			return t("exploreAppTypeForm", "Form");
		default:
			return undefined;
	}
}

function kindCounts(t: StoreT, apps: number, packages: number): string {
	const appText = t("exploreAppCount", {
		count: apps,
		defaultValue_one: "{{count}} app",
		defaultValue_other: "{{count}} apps",
	});
	const packageText = t("explorePackageCount", {
		count: packages,
		defaultValue_one: "{{count}} package",
		defaultValue_other: "{{count}} packages",
	});
	if (apps > 0 && packages > 0) return `${appText} · ${packageText}`;
	return packages > 0 ? packageText : appText;
}

function packageCategoryLabel(
	t: StoreT,
	category: WasmPackageCategory,
): string {
	switch (category) {
		case "DOCUMENT_PROCESSING":
			return t("packageCategoryDocumentProcessing", "Document Processing");
		case "DATA_TRANSFORMATION":
			return t("packageCategoryDataTransformation", "Data Transformation");
		case "WORKFLOW_AUTOMATION":
			return t("packageCategoryWorkflowAutomation", "Workflow Automation");
		case "COMMUNICATION":
			return t("packageCategoryCommunication", "Communication");
		case "ANALYTICS_REPORTING":
			return t("packageCategoryAnalyticsReporting", "Analytics & Reporting");
		case "FINANCE_BILLING":
			return t("packageCategoryFinanceBilling", "Finance & Billing");
		case "COMPLIANCE_REGULATORY":
			return t(
				"packageCategoryComplianceRegulatory",
				"Compliance & Regulatory",
			);
		case "HR_PEOPLE":
			return t("packageCategoryHrPeople", "HR & People");
		case "AI_ML":
			return t("packageCategoryAiMl", "AI & ML");
		case "INTEGRATION_CONNECTORS":
			return t(
				"packageCategoryIntegrationConnectors",
				"Integrations & Connectors",
			);
		case "SECURITY_IDENTITY":
			return t("packageCategorySecurityIdentity", "Security & Identity");
		case "DEVOPS":
			return t("packageCategoryDevops", "DevOps");
		case "IOT_INDUSTRIAL":
			return t("packageCategoryIotIndustrial", "IoT & Industrial");
		case "ROBOTICS_PHYSICAL_AI":
			return t("packageCategoryRoboticsPhysicalAi", "Robotics & Physical AI");
		case "GAMING_SIMULATION":
			return t("packageCategoryGamingSimulation", "Gaming & Simulation");
		case "HEALTHCARE":
			return t("packageCategoryHealthcare", "Healthcare");
		case "VETERINARY":
			return t("packageCategoryVeterinary", "Veterinary");
		case "LEGAL":
			return t("packageCategoryLegal", "Legal");
		case "MANUFACTURING":
			return t("packageCategoryManufacturing", "Manufacturing");
		case "AGRICULTURE":
			return t("packageCategoryAgriculture", "Agriculture");
		case "REAL_ESTATE":
			return t("packageCategoryRealEstate", "Real Estate");
		case "LOGISTICS":
			return t("packageCategoryLogistics", "Logistics");
		case "ENERGY":
			return t("packageCategoryEnergy", "Energy");
		case "CONSTRUCTION_TRADES":
			return t("packageCategoryConstructionTrades", "Construction & Trades");
		case "EDUCATION":
			return t("packageCategoryEducation", "Education");
		case "GOVERNMENT_DEFENSE":
			return t("packageCategoryGovernmentDefense", "Government & Defense");
		case "ECOMMERCE":
			return t("packageCategoryEcommerce", "E-commerce");
		case "INSURANCE":
			return t("packageCategoryInsurance", "Insurance");
		case "TELECOM":
			return t("packageCategoryTelecom", "Telecom");
		case "SCIENTIFIC_ENGINEERING":
			return t("packageCategoryScientificEngineering", "Science & Engineering");
		case "GEOSPATIAL":
			return t("packageCategoryGeospatial", "Geospatial");
		case "MEDIA_CONTENT":
			return t("packageCategoryMediaContent", "Media & Content");
		case "OTHER":
			return t("packageCategoryOther", "Other");
	}
}
