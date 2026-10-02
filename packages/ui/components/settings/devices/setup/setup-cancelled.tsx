"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { CircleX, KeyRound } from "lucide-react";
import type { Ref } from "react";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StatusChip } from "../primitives/status-chip";
import { WizardStepHeader } from "../primitives/wizard";
import { useSetup } from "./setup-context";
import { Mono } from "./setup-parts";
import { packageFile } from "./setup-state";

/** What a cancelled setup leaves behind: a dead package, a free slot, and the keys on this computer. */
export function SetupCancelled({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, limits, leaveLink } = useSetup();
	const at = draft.cancelledAt ?? Math.floor(time.nowS);
	return (
		<>
			<WizardStepHeader
				headingRef={headingRef}
				title={t("setup.cancelled.title", "Setup cancelled")}
				className="sr-only"
			/>
			<Block
				icon={CircleX}
				title={t("setup.cancelled.block", "Cancelled setup")}
				summary={
					<StatusChip tone="outline" icon={CircleX}>
						{enumLabel(t, "enrollment", "cancelled")}
					</StatusChip>
				}
				stamp={
					<FreshnessStamp
						source="hub"
						age="current"
						noFail
						observedAt={at}
						text={t("setup.cancelled.stamp", "cancelled {{ago}}", {
							ago: time.ago(at),
						})}
					/>
				}
			>
				<KeyValueList>
					<KvRow label={t("setup.cancelled.when", "Cancelled")}>
						{time.at(at)}
					</KvRow>
					<KvRow label={t("setup.field.package", "Package")}>
						<Trans
							t={t}
							i18nKey="setup.cancelled.package"
							defaults="<1/> no longer works, even if someone starts it."
							components={{ 1: <Mono>{packageFile(draft.name)}</Mono> }}
						/>
					</KvRow>
					<KvRow label={t("setup.cancelled.limit", "Your limit")}>
						{limits.maxPending === undefined
							? t(
									"setup.cancelled.limitPlain",
									"Its unused-package slot is free again. It still counts toward today's packages.",
								)
							: t(
									"setup.cancelled.limitText",
									"1 of your {{max, number}} unused-package slots is free again. It still counts toward today's packages.",
									{ max: limits.maxPending },
								)}
					</KvRow>
					<KvRow label={t("setup.cancelled.keys", "Keys")}>
						<Trans
							t={t}
							i18nKey="setup.cancelled.keysText"
							defaults="The owner keys made for <1/> stay on this computer until you delete them."
							components={{ 1: <Mono>{draft.name}</Mono> }}
						/>
					</KvRow>
				</KeyValueList>
				<DvButton size="sm" icon={KeyRound} className="self-start" asChild>
					<a {...leaveLink({ screen: "keys" })}>
						{t("setup.save.backup.openKeys", "Open Keys & recovery")}
					</a>
				</DvButton>
			</Block>
		</>
	);
}
