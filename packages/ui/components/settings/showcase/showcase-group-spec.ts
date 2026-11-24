"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { eventKind } from "../../store/showcase/model";
import type {
	ShowcaseBoolPath,
	ShowcaseListKey,
} from "./showcase-settings-model";

export interface ShowcaseSwitchSpec {
	path: ShowcaseBoolPath;
	label: string;
	hint?: string;
}

export interface ShowcaseListSpec {
	key: ShowcaseListKey;
	heading: string;
	empty: string;
	items: { id: string; label: string; hint?: string }[];
}

export interface ShowcaseGroupSpec {
	id: string;
	master: ShowcaseSwitchSpec;
	list?: ShowcaseListSpec;
	subs: ShowcaseSwitchSpec[];
}

export interface ShowcaseEventItem {
	id: string;
	name: string;
	type: string;
}

export function useShowcaseGroupSpecs(
	tables: readonly string[],
	events: readonly ShowcaseEventItem[],
): ShowcaseGroupSpec[] {
	const { t } = useTranslation("store");
	return useMemo<ShowcaseGroupSpec[]>(
		() => [
			{
				id: "map",
				master: {
					path: "map",
					label: t("showcaseSettingMap", "App map"),
					hint: t(
						"showcaseSettingMapHint",
						"How triggers, flows, data and pages connect",
					),
				},
				subs: [],
			},
			{
				id: "triggers",
				master: {
					path: "triggers.enabled",
					label: t("showcaseSettingTriggers", "Ways to use it"),
					hint: t(
						"showcaseSettingTriggersHint",
						"The events visitors can start this app with",
					),
				},
				list: {
					key: "events",
					heading: t("showcaseSettingEvents", "Events to show"),
					empty: t("showcaseSettingNoEvents", "This app has no events yet."),
					items: events.map((event) => ({
						id: event.id,
						label: event.name,
						hint: (() => {
							const [key, fallback] = eventKind(event.type).label;
							return t(key as "showcaseEventOther", fallback);
						})(),
					})),
				},
				subs: [
					{
						path: "triggers.technical",
						label: t("showcaseSettingTechnical", "Technical details"),
						hint: t(
							"showcaseSettingTechnicalHint",
							"HTTP method and path, cron schedules",
						),
					},
				],
			},
			{
				id: "data",
				master: {
					path: "data.enabled",
					label: t("showcaseSettingData", "Data"),
					hint: t("showcaseSettingDataHint", "The tables behind this app"),
				},
				list: {
					key: "tables",
					heading: t("showcaseSettingTables", "Tables to show"),
					empty: t("showcaseSettingNoTables", "This app has no tables yet."),
					items: tables.map((name) => ({ id: name, label: name })),
				},
				subs: [
					{
						path: "data.row_counts",
						label: t("showcaseSettingRowCounts", "Row counts"),
					},
					{
						path: "data.schema",
						label: t("showcaseSettingColumns", "Columns and types"),
					},
					{
						path: "data.samples",
						label: t("showcaseSettingSamples", "Sample rows"),
						hint: t(
							"showcaseSettingSamplesHint",
							"Up to 5 rows. Personal columns stay hidden.",
						),
					},
					{
						path: "data.ontology",
						label: t("showcaseSettingOntology", "Object types and links"),
					},
					{
						path: "data.sources",
						label: t("showcaseSettingSources", "External connections"),
						hint: t(
							"showcaseSettingSourcesHint",
							"Name and driver only, never hosts or credentials",
						),
					},
				],
			},
			{
				id: "pages",
				master: {
					path: "pages.enabled",
					label: t("showcaseSettingPages", "Pages"),
				},
				subs: [
					{
						path: "pages.previews",
						label: t("showcaseSettingPagePreviews", "Page previews"),
						hint: t(
							"showcaseSettingPagePreviewsHint",
							"A wireframe of each page's layout, without text or data",
						),
					},
				],
			},
			{
				id: "widgets",
				master: {
					path: "widgets.enabled",
					label: t("showcaseSettingWidgets", "Home widgets"),
				},
				subs: [],
			},
			{
				id: "templates",
				master: {
					path: "templates.enabled",
					label: t("showcaseSettingTemplates", "Templates"),
				},
				subs: [],
			},
			{
				id: "flows",
				master: {
					path: "flows.enabled",
					label: t("showcaseSettingFlows", "Flows"),
					hint: t(
						"showcaseSettingFlowsHint",
						"Names and node counts. Off, the map goes straight from trigger to result.",
					),
				},
				subs: [],
			},
			{
				id: "hood",
				master: {
					path: "hood.enabled",
					label: t("showcaseSettingHood", "Under the hood"),
				},
				subs: [
					{
						path: "hood.runtime",
						label: t("showcaseSettingRuntime", "Where it runs"),
					},
					{
						path: "hood.models",
						label: t("showcaseSettingModels", "AI models"),
					},
					{
						path: "hood.packages",
						label: t(
							"showcaseSettingPackages",
							"Node packages and permissions",
						),
					},
					{
						path: "hood.accounts",
						label: t("showcaseSettingAccounts", "Accounts to connect"),
					},
				],
			},
			{
				id: "activity",
				master: {
					path: "activity.enabled",
					label: t("showcaseSettingActivity", "Activity"),
				},
				subs: [
					{
						path: "activity.runs",
						label: t("showcaseSettingRuns", "Run counts"),
						hint: t("showcaseSettingRunsHint", "Last 30 days"),
					},
					{
						path: "activity.versions",
						label: t("showcaseSettingVersions", "Latest release"),
					},
				],
			},
		],
		[t, tables, events],
	);
}
