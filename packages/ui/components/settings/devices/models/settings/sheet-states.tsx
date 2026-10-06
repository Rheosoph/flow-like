"use client";

import { useTranslation } from "@flow-like/locales";
import type { ModelsOverview } from "../../../../../lib/device-management/models";
import type { ObserveTarget } from "../../observe/use-observe-target";
import { StateView } from "../../primitives/state-view";
import {
	ModelsGateState,
	ModelsReadFailed,
	OlderAgentNotice,
} from "../models-states";
import type { ModelsAccess, ModelsRead } from "../use-models";

/** What a model sheet shows while the device's overview can't be read: why, or that it is being read. */
export function OverviewPending({
	target,
	access,
	overview,
}: Readonly<{
	target: ObserveTarget;
	access: ModelsAccess;
	overview: ModelsRead<ModelsOverview>;
}>) {
	const { t } = useTranslation("devices");
	if (overview.unsupported) return <OlderAgentNotice target={target} />;
	if (!access.gate.ok)
		return <ModelsGateState target={target} gate={access.gate} />;
	if (overview.failure)
		return (
			<ModelsReadFailed
				target={target}
				failure={overview.failure}
				onRetry={overview.refetch}
			/>
		);
	return (
		<StateView
			kind="loading"
			rows={4}
			title={t(
				"devices:models.states.reading",
				"Reading models from {{device}}…",
				{ device: target.name },
			)}
		/>
	);
}
