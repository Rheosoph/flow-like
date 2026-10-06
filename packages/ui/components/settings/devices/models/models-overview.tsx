"use client";

import { useTranslation } from "@flow-like/locales";
import { Lightbulb } from "lucide-react";
import { useMemo } from "react";
import type { ModelsOverview } from "../../../../lib/device-management/models";
import { gateView } from "../device/use-device-page";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import {
	type AttentionEntry,
	AttentionList,
} from "../primitives/attention-list";
import { Block } from "../primitives/block";
import {
	FreshnessStamp,
	type FreshnessStampProps,
} from "../primitives/freshness-stamp";
import { Headline } from "../primitives/headline";
import { headlineCopy } from "./models-copy";
import { TIER_SEVERITY, modelsHeadline } from "./models-view";
import {
	type RecommendationFix,
	recommendationCopy,
	recommendationFix,
	recommendationId,
	recommendationIds,
} from "./recommendation-copy";
import { useModelsAction } from "./use-models-action";

type AreaTime = ReturnType<typeof useAreaTime>;

export function ModelsHeadline({
	overview,
	device,
}: Readonly<{ overview: ModelsOverview; device: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const copy = headlineCopy(t, modelsHeadline(overview), {
		device,
		locale: time.locale,
	});
	return <Headline lead={copy.lead} rest={copy.rest} names={[device]} />;
}

/** The one-click fix the device attached, as the entry's action. */
function fixAction(
	t: DevicesT,
	time: AreaTime,
	fix: RecommendationFix | undefined,
): Pick<AttentionEntry, "action"> {
	if (!fix) return {};
	return {
		action: {
			label: fix.label,
			gate: gateView(t, time, fix.command.gate)?.gate ?? null,
			onSelect: () => void fix.command.run(),
		},
	};
}

/** The device's recommendations in now/soon/later tiers; the copy is `recommendation-copy.ts`. */
export function RecommendationsBlock({
	deviceId,
	overview,
	device,
	stamp,
}: Readonly<{
	deviceId: string;
	overview: ModelsOverview;
	device: string;
	stamp: FreshnessStampProps;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useModelsAction(deviceId);
	const items = useMemo<AttentionEntry[]>(() => {
		const names = new Map(
			overview.models.map((model) => [model.id, model.display_name]),
		);
		const context = { device, modelName: (id: string) => names.get(id) };
		const ids = recommendationIds(overview.recommendations);
		return overview.recommendations.map((recommendation, index) => {
			const copy = recommendationCopy(t, recommendation, context);
			const fix = recommendationFix(t, recommendation, overview, actions);
			return {
				id: ids[index] ?? recommendationId(recommendation),
				severity: TIER_SEVERITY[recommendation.tier],
				sentence: copy.sentence,
				names: copy.names,
				conditionKey: recommendation.code,
				stamp,
				...fixAction(t, time, fix),
			};
		});
	}, [t, time, actions, overview, device, stamp]);
	// Device-wide ones (a hidden container GPU, a full disk) matter most before the first model.
	if (!overview.models.length && !overview.recommendations.length) return null;
	return (
		<Block
			id="models-attention"
			icon={Lightbulb}
			title={t("devices:models.overview.recommendations", "Recommendations")}
			count={items.length}
			stamp={<FreshnessStamp {...stamp} />}
			flush
		>
			<AttentionList
				items={items}
				base={stamp}
				emptyText={t(
					"devices:models.overview.noRecommendations",
					"Nothing to improve: {{device}} serves its models as well as it can.",
					{ device },
				)}
			/>
		</Block>
	);
}
