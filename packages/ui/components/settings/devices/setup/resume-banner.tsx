"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { ArrowRight, Hourglass } from "lucide-react";
import { useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { DvButton } from "../primitives/dv-button";
import type { RouteLinkProps } from "../routing/use-devices-route";
import { Mono } from "./setup-parts";
import type { WaitingSetup } from "./setup-state";

/**
 * On a new setup: an earlier one from this window is still waiting for its
 * device. Its package can't be downloaded again, but the start instructions
 * and the waiting step can be opened again.
 */
export function ResumeBanner({
	setup,
	link,
	onDismiss,
}: Readonly<{
	setup: WaitingSetup;
	link: RouteLinkProps;
	onDismiss(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const created = setup.draft.created;
	if (!created) return null;
	return (
		<Banner
			tone="info"
			icon={Hourglass}
			title={
				<Trans
					t={t}
					i18nKey="setup.resume.title"
					defaults="<1/> is still waiting to be started."
					components={{ 1: <Mono>{setup.draft.name}</Mono> }}
				/>
			}
			actions={
				<>
					<DvButton size="sm" icon={ArrowRight} asChild>
						<a {...link}>{t("setup.resume.continue", "Continue that setup")}</a>
					</DvButton>
					<DvButton size="sm" variant="ghost" onClick={onDismiss}>
						{t("setup.resume.dismiss", "Dismiss")}
					</DvButton>
				</>
			}
		>
			{t(
				"setup.resume.text",
				"You made its package {{when}} in this window. It works until {{until}}; you're starting another setup here.",
				{
					when: time.ago(created.createdAt),
					until: time.at(created.expiresAt),
				},
			)}
		</Banner>
	);
}

/** After a link or Back asked for a step that the registered device has locked. */
export function LockedStepsBanner({
	name,
	onDismiss,
}: Readonly<{ name: string; onDismiss(): void }>) {
	const { t } = useTranslation("devices");
	return (
		<Banner
			tone="info"
			title={t("setup.locked.title", "The earlier steps are locked.")}
			actions={
				<DvButton size="sm" variant="ghost" onClick={onDismiss}>
					{t("setup.resume.dismiss", "Dismiss")}
				</DvButton>
			}
		>
			<Trans
				t={t}
				i18nKey="setup.locked.text"
				defaults="The package for <1/> is built, so its name, platform and password can't change. To change them, cancel this setup and start a new one."
				components={{ 1: <Mono>{name}</Mono> }}
			/>
		</Banner>
	);
}
