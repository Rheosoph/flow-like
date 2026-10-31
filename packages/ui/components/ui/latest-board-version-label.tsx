"use client";

import { useTranslation } from "@flow-like/locales";
import type { FlowVersionState } from "../../lib/device-management/hub/endpoints";

export function LatestBoardVersionLabel({
	state,
}: { state: FlowVersionState | null | undefined }) {
	const { t } = useTranslation("flow");
	if (!state) return t("latestDraft", "Latest (draft)");
	if (state.current) {
		return t("latestPublishedVersion", "Latest (v{{version}})", {
			version: state.current.join("."),
		});
	}
	if (state.newest) {
		return t(
			"latestChangedDraft",
			"Latest (draft has changes; published v{{version}})",
			{ version: state.newest.join(".") },
		);
	}
	return t("latestUnpublishedDraft", "Latest (unpublished draft)");
}
