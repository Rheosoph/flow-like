"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { gateCopy } from "../copy/gate-copy";
import { checkLabel } from "../hub/readiness-list";
import { releaseCheckText } from "../hub/release-copy";
import { useAreaTime } from "../primitives/area-context";
import { dayText } from "../primitives/day";
import { Headline } from "../primitives/headline";
import { useSetup } from "./setup-context";
import { Mono, startByReleaseText } from "./setup-parts";
import {
	CREATE_STEP,
	type CreatedSetup,
	SAVE_STEP,
	START_STEP,
	startBy,
} from "./setup-state";
import { WAIT_POLL_MS } from "./use-setup-wait";

/* SPEC §4.24: the one conclusion sentence of the step. Steps 1–3 are plain forms and have none. */

function CancelledHeadline() {
	const { t } = useTranslation("devices");
	const { draft } = useSetup();
	return (
		<Headline
			lead={
				<Trans
					t={t}
					i18nKey="setup.headline.cancelled"
					defaults="The setup for <1/> is cancelled."
					components={{ 1: <Mono>{draft.name}</Mono> }}
				/>
			}
			rest={t(
				"setup.headline.cancelledRest",
				"Its package no longer works and its slot is free again.",
			)}
		/>
	);
}

/** Step 0: ready, or the one reason it is not. */
function CheckHeadline() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { host, checks, limits } = useSetup();
	const { readiness, release, gate } = checks;

	if (readiness.state === "checking") return null;
	if (readiness.state === "failed" || readiness.error)
		return (
			<Headline
				lead={t(
					"setup.headline.unanswered",
					"{{host}} isn't answering right now.",
					{ host },
				)}
				rest={
					readiness.state === "loaded"
						? t(
								"setup.headline.unansweredRest",
								"Its checks were last read at {{time}}. You can prepare the setup; creating the package waits until the hub answers again.",
								{ time: time.clock(Math.floor(readiness.checkedAt / 1000)) },
							)
						: t(
								"setup.headline.unansweredFirst",
								"The hub checks couldn't be read. Nothing was created.",
							)
				}
			/>
		);
	const failing = readiness.data.checks.find((check) => !check.ready);
	if (failing)
		return (
			<Headline
				lead={t(
					"setup.headline.notReady",
					"{{host}} can't set up devices right now.",
					{ host },
				)}
				rest={t(
					"setup.headline.notReadyRest",
					"{{check}} isn't ready, so a new device couldn't be set up. Nothing was created and no slot is used.",
					{ check: checkLabel(t, failing.id) },
				)}
			/>
		);
	if (release.state === "rejected" && release.check === "expired")
		return (
			<Headline
				lead={t(
					"setup.headline.ranOut",
					"The hub's agent release has run out.",
				)}
				rest={
					release.facts
						? t(
								"setup.headline.ranOutRest",
								"It ran out on {{date}}, so no package can be built until the hub operator publishes or renews a release. Nothing was created.",
								{ date: dayText(time, release.facts.expires_at) },
							)
						: undefined
				}
			/>
		);
	if (release.state === "rejected")
		return (
			<Headline
				lead={t("setup.headline.rejected", "The agent release failed a check.")}
				rest={t(
					"setup.headline.rejectedRest",
					"The reason: {{reason}}. No package can be built from it, and nothing was created.",
					{ reason: releaseCheckText(t, release.check) },
				)}
			/>
		);
	if (release.state === "missing")
		return (
			<Headline
				lead={t(
					"setup.check.noRelease.title",
					"This hub has no signed agent releases.",
				)}
				rest={t(
					"setup.check.noRelease.text",
					"Without them no setup package can be built. Ask your hub operator to publish signed releases.",
				)}
			/>
		);
	if (!gate.ok)
		return (
			<Headline
				lead={gateCopy(t, gate).title}
				rest={t(
					"setup.headline.limitRest",
					"A new package can't be made until there is room again. Nothing was created.",
				)}
			/>
		);
	if (release.state !== "verified") return null;
	const version = release.release.manifest.release_version;
	const checked = readiness.data.checks.length;
	const { maxDevices, activeDevices, pending } = limits;
	const room =
		maxDevices !== undefined &&
		activeDevices !== undefined &&
		pending !== undefined
			? maxDevices - activeDevices - pending
			: undefined;
	return (
		<Headline
			lead={t("setup.headline.ready", "{{host}} is ready for a new device.", {
				host,
			})}
			rest={
				room === undefined
					? t(
							"setup.headline.readyRestPlain",
							"All {{checks, number}} hub checks pass and agent {{version}} is verified.",
							{ checks: checked, version },
						)
					: t(
							"setup.headline.readyRest",
							"All {{checks, number}} hub checks pass, agent {{version}} is verified, and you can add {{count, number}} more devices.",
							{ checks: checked, version, count: room },
						)
			}
		/>
	);
}

/** Step 4: nothing before the run; then running, failed or ready. */
function CreateHeadline() {
	const { t } = useTranslation("devices");
	const { draft, create } = useSetup();
	const name = <Mono>{draft.name}</Mono>;
	if (create.run.status === "failed")
		return (
			<Headline
				lead={
					<Trans
						t={t}
						i18nKey="setup.headline.createFailed"
						defaults="The package for <1/> couldn't be created."
						components={{ 1: name }}
					/>
				}
				rest={
					create.run.failure?.kind === "cancel_failed"
						? t(
								"setup.headline.createFailedKept",
								"The hub still holds the half-made setup. Cancel it before you try again.",
							)
						: t(
								"setup.headline.createFailedRest",
								"Nothing is left half-made. Try again when the cause below is fixed.",
							)
				}
			/>
		);
	if (create.run.status === "running")
		return (
			<Headline
				lead={
					<Trans
						t={t}
						i18nKey="setup.headline.creating"
						defaults="Creating the package for <1/>."
						components={{ 1: name }}
					/>
				}
				rest={t(
					"setup.headline.creatingRest",
					"Keep this window open; it usually takes under a minute.",
				)}
			/>
		);
	if (!draft.created) return null;
	return (
		<Headline
			lead={
				<Trans
					t={t}
					i18nKey="setup.headline.created"
					defaults="The package for <1/> is ready."
					components={{ 1: name }}
				/>
			}
			rest={t(
				"setup.headline.createdRest",
				"The device is registered with the hub and waits for its first start.",
			)}
		/>
	);
}

function SaveHeadline({ created }: Readonly<{ created: CreatedSetup }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, built } = useSetup();
	const name = <Mono>{draft.name}</Mono>;
	return built ? (
		<Headline
			lead={
				<Trans
					t={t}
					i18nKey="setup.headline.save"
					defaults="The package for <1/> works until {{until}}."
					values={{ until: time.at(startBy(created)) }}
					components={{ 1: name }}
				/>
			}
			rest={
				created.releaseEndsAt === undefined
					? t(
							"setup.headline.saveRest",
							"Start it on the device before then. It can't be downloaded again once you leave this setup.",
						)
					: startByReleaseText(t, time, created.releaseEndsAt, true)
			}
		/>
	) : (
		<Headline
			lead={
				<Trans
					t={t}
					i18nKey="setup.headline.saveGone"
					defaults="The package for <1/> was built before this window was opened."
					components={{ 1: name }}
				/>
			}
			rest={t(
				"setup.headline.saveGoneRest",
				"It can't be downloaded again from here. If you have it, carry on; if not, cancel this setup and create a new one.",
			)}
		/>
	);
}

/** Step 6 only says something for a setup opened from Pending setups. */
function StartHeadline({ created }: Readonly<{ created: CreatedSetup }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft } = useSetup();
	if (!draft.resumed) return null;
	return (
		<Headline
			lead={
				<Trans
					t={t}
					i18nKey="setup.headline.resumed"
					defaults="<1/> is waiting to be started."
					components={{ 1: <Mono>{draft.name}</Mono> }}
				/>
			}
			rest={t(
				"setup.headline.resumedRest",
				"Its package was made {{when}} and works until {{until}}. Only the steps from here on apply.",
				{
					when: time.at(created.createdAt),
					until: time.at(startBy(created)),
				},
			)}
		/>
	);
}

/** Step 7: expired, checked in, registered, or still waiting. */
function WaitHeadline({ created }: Readonly<{ created: CreatedSetup }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, expired } = useSetup();
	const name = <Mono>{draft.name}</Mono>;
	const until = time.at(startBy(created));
	if (expired)
		return (
			<Headline
				lead={
					<Trans
						t={t}
						i18nKey="setup.headline.expired"
						defaults="The package for <1/> expired unused on {{when}}."
						values={{ when: until }}
						components={{ 1: name }}
					/>
				}
				rest={t(
					"setup.headline.expiredRest",
					"It no longer works and nothing was installed. Create a new one to set the device up.",
				)}
			/>
		);
	if (draft.checkedInAt !== undefined)
		return (
			<Headline
				lead={
					<Trans
						t={t}
						i18nKey="setup.headline.done"
						defaults="<1/> is set up and checked in at {{time}}."
						values={{ time: time.clock(draft.checkedInAt) }}
						components={{ 1: name }}
					/>
				}
				rest={
					created.outcome === "saved"
						? t(
								"setup.headline.doneRestSaved",
								"Its owner keys are on this computer and backed up to your account.",
							)
						: t(
								"setup.headline.doneRest",
								"Its owner keys are on this computer.",
							)
				}
			/>
		);
	if (draft.registeredAt !== undefined)
		return (
			<Headline
				lead={
					<Trans
						t={t}
						i18nKey="setup.headline.registered"
						defaults="<1/> registered with the hub."
						components={{ 1: name }}
					/>
				}
				rest={t(
					"setup.headline.registeredRest",
					"Waiting for its first check-in…",
				)}
			/>
		);
	return (
		<Headline
			lead={
				<Trans
					t={t}
					i18nKey="setup.headline.waiting"
					defaults="Waiting for <1/> to start."
					components={{ 1: name }}
				/>
			}
			rest={
				created.releaseEndsAt === undefined
					? t(
							"setup.headline.waitingRest",
							"Start the package on the device before {{until}}. This page checks the hub every {{count, number}} s.",
							{ until, count: WAIT_POLL_MS / 1000 },
						)
					: startByReleaseText(t, time, created.releaseEndsAt, true)
			}
		/>
	);
}

export function SetupHeadline() {
	const { draft, step } = useSetup();
	const { created } = draft;
	if (draft.cancelledAt) return <CancelledHeadline />;
	if (step === 0) return <CheckHeadline />;
	if (step === CREATE_STEP) return <CreateHeadline />;
	if (!created) return null;
	if (step === SAVE_STEP) return <SaveHeadline created={created} />;
	if (step === START_STEP) return <StartHeadline created={created} />;
	return step > START_STEP ? <WaitHeadline created={created} /> : null;
}
