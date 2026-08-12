"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { useFeatures } from "../../../hooks/use-features";
import { useInvoke } from "../../../hooks/use-invoke";
import type { IApp, IEvent, IMetadata } from "../../../lib";
import { IAppVisibility } from "../../../lib";
import { useBackend } from "../../../state/backend-state";
import type { ProjectRunHealth } from "./use-project-runs";

export type SignalTone = "critical" | "warning" | "info";

export interface AttentionSignal {
	id: string;
	tone: SignalTone;
	/** Short enough to sit in a chip. */
	label: string;
	/** The named thing the signal is about, emphasised in the chip. */
	subject?: string;
	/** Label for the action that resolves it. */
	actionLabel: string;
	/** Where the fix lives — a route, or an inspector panel. */
	href?: string;
	panel?: InspectorPanel;
	/** Which Launch Path stage this belongs to, when it maps to one. */
	stage?: number;
}

export type InspectorPanel =
	| "identity"
	| "access"
	| "listing"
	| "compliance"
	| "release"
	| "advanced";

export interface AiActStatus {
	available: boolean;
	isLoading: boolean;
	hasAssessment: boolean;
	riskCategory: string | null;
	conformityScore: number | null;
	blocked: boolean;
}

interface QuestionnaireSummary {
	classification: {
		riskCategory: string;
		conformityScore: number | null;
		blocked: boolean;
	};
	hasAssessment: boolean;
}

const ONLINE_VISIBILITIES = [
	IAppVisibility.Public,
	IAppVisibility.Prototype,
	IAppVisibility.PublicRequestAccess,
];

export function isOnlineVisibility(visibility: IAppVisibility): boolean {
	return ONLINE_VISIBILITIES.includes(visibility);
}

/**
 * Shares the wizard's query key so the dashboard badge and the wizard itself
 * never disagree, and so opening the wizard costs no extra request.
 */
export function useAiActStatus(
	appId: string | undefined,
	visibility: IAppVisibility | undefined,
): AiActStatus {
	const backend = useBackend();
	const features = useFeatures();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const enabled =
		!!appId &&
		!!profile.data &&
		!!features.data?.ai_act &&
		visibility !== IAppVisibility.Offline;

	const questionnaire = useQuery<QuestionnaireSummary>({
		queryKey: ["ai-act", "questionnaire", appId],
		enabled,
		staleTime: 5 * 60 * 1000,
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<QuestionnaireSummary>(
				profile.data.hub_profile,
				`apps/${appId}/ai-act/questionnaire`,
			);
		},
	});

	return useMemo(
		() => ({
			available: enabled,
			isLoading: questionnaire.isLoading,
			hasAssessment: questionnaire.data?.hasAssessment ?? false,
			riskCategory: questionnaire.data?.classification?.riskCategory ?? null,
			conformityScore:
				questionnaire.data?.classification?.conformityScore ?? null,
			blocked: questionnaire.data?.classification?.blocked ?? false,
		}),
		[enabled, questionnaire.data, questionnaire.isLoading],
	);
}

export interface ListingChecklistItem {
	id: string;
	label: string;
	done: boolean;
}

/** The nine things a store listing needs, checked against real metadata. */
export function useListingChecklist(
	app: IApp | undefined,
	metadata: IMetadata | undefined,
): { items: ListingChecklistItem[]; done: number; total: number } {
	return useMemo(() => {
		const items: ListingChecklistItem[] = [
			{
				id: "name",
				label: "Name & summary",
				done: !!metadata?.name && !!metadata?.description,
			},
			{ id: "icon", label: "Icon", done: !!metadata?.icon },
			{ id: "banner", label: "Banner image", done: !!metadata?.thumbnail },
			{
				id: "long",
				label: "Full description",
				done: (metadata?.long_description?.trim().length ?? 0) > 0,
			},
			{
				id: "category",
				label: "Categories",
				done: !!app?.primary_category,
			},
			{
				id: "tags",
				label: "Tags",
				done: (metadata?.tags?.length ?? 0) > 0,
			},
			{
				id: "links",
				label: "Support & docs links",
				done: !!metadata?.docs_url || !!metadata?.support_url,
			},
			{ id: "version", label: "Version", done: !!app?.version },
			{
				id: "changelog",
				label: "Changelog",
				done: (app?.changelog?.trim().length ?? 0) > 0,
			},
		];
		return {
			items,
			done: items.filter((item) => item.done).length,
			total: items.length,
		};
	}, [app, metadata]);
}

export interface ProjectSignalsInput {
	appId: string;
	app: IApp | undefined;
	events: IEvent[] | undefined;
	runs: ProjectRunHealth;
	aiAct: AiActStatus;
	listingDone: number;
	listingTotal: number;
	boardNames: Map<string, string>;
}

/**
 * Turns real project state into the ranked "needs you" queue. Every entry is
 * derived from something observable — a failed run, a paused event, a missing
 * assessment. Nothing is invented, so an untouched project legitimately has an
 * empty queue.
 */
export function useProjectSignals({
	appId,
	app,
	events,
	runs,
	aiAct,
	listingDone,
	listingTotal,
	boardNames,
}: ProjectSignalsInput): AttentionSignal[] {
	return useMemo(() => {
		const signals: AttentionSignal[] = [];

		for (const [boardId, health] of runs.byBoard) {
			if (health.failed === 0) continue;
			signals.push({
				id: `runs-failed-${boardId}`,
				tone: "critical",
				label: `${health.failed} run${health.failed === 1 ? "" : "s"} failed`,
				subject: boardNames.get(boardId) ?? "Deleted flow",
				actionLabel: "Open runs",
				href: `/flow?id=${boardId}&app=${appId}`,
				stage: 3,
			});
		}

		const pausedEvents = (events ?? []).filter((event) => !event.active);
		for (const event of pausedEvents.slice(0, 2)) {
			signals.push({
				id: `event-paused-${event.id}`,
				tone: "warning",
				label: "Trigger is paused",
				subject: event.name,
				actionLabel: "Manage",
				href: `/library/config/pages?id=${appId}`,
				stage: 2,
			});
		}

		if (aiAct.available && !aiAct.hasAssessment) {
			signals.push({
				id: "ai-act-incomplete",
				tone: "warning",
				label: "EU AI Act assessment not submitted",
				actionLabel: "Answer",
				panel: "compliance",
				stage: 6,
			});
		}

		if (aiAct.blocked) {
			signals.push({
				id: "ai-act-blocked",
				tone: "critical",
				label: "A prohibited use is selected — publication is blocked",
				actionLabel: "Review",
				panel: "compliance",
				stage: 6,
			});
		}

		if (app?.visibility === IAppVisibility.Private) {
			signals.push({
				id: "private-locks-team",
				tone: "info",
				label: "Team and Roles need Prototype",
				actionLabel: "Change visibility",
				panel: "access",
				stage: 4,
			});
		}

		if (
			app &&
			isOnlineVisibility(app.visibility) &&
			listingDone < listingTotal
		) {
			signals.push({
				id: "listing-incomplete",
				tone: "info",
				label: `Store listing ${listingDone} of ${listingTotal} complete`,
				actionLabel: "Finish",
				panel: "listing",
				stage: 5,
			});
		}

		const order: Record<SignalTone, number> = {
			critical: 0,
			warning: 1,
			info: 2,
		};
		return signals.sort((a, b) => order[a.tone] - order[b.tone]);
	}, [
		appId,
		app,
		events,
		runs.byBoard,
		aiAct,
		listingDone,
		listingTotal,
		boardNames,
	]);
}
