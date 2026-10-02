"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Boxes,
	Container,
	Cpu,
	Info,
	Laptop,
	type LucideIcon,
	OctagonX,
	Server,
	Terminal,
} from "lucide-react";
import { type Ref, useState } from "react";
import type { ReleaseTarget } from "../../../../../lib/device-management/package";
import { humanFileSize } from "../../../../../lib/utils";
import { enumLabel } from "../../copy/enum-labels";
import type { DevicesT } from "../../primitives/area-context";
import { CommandBlock } from "../../primitives/command-block";
import { DvButton } from "../../primitives/dv-button";
import { ChoiceCards, type ChoiceOption } from "../../primitives/form-fields";
import { GateInline } from "../../primitives/gate-notice";
import { StateView } from "../../primitives/state-view";
import { WizardStepHeader } from "../../primitives/wizard";
import { useSetup } from "../setup-context";
import { FieldNote, Mono } from "../setup-parts";
import {
	STEP_COUNT,
	type SetupMode,
	type TargetOption,
	defaultMode,
	modeAvailable,
} from "../setup-state";

const TARGET_ICON: Record<ReleaseTarget, LucideIcon> = {
	"x86_64-unknown-linux-gnu": Server,
	"aarch64-unknown-linux-gnu": Cpu,
	"x86_64-apple-darwin": Laptop,
	"aarch64-apple-darwin": Laptop,
};

const MODE_ICON: Record<SetupMode, LucideIcon> = {
	binary: Terminal,
	docker: Container,
	both: Boxes,
};
const MODES: readonly SetupMode[] = ["binary", "docker", "both"];

/** Decimal megabytes, as download pages state them. */
export const megabytes = (bytes: number) => humanFileSize(bytes, true);

function targetHint(t: DevicesT, target: ReleaseTarget): string {
	switch (target) {
		case "x86_64-unknown-linux-gnu":
			return t(
				"devices:setup.platform.hint.linuxX64",
				"Most servers, mini PCs and virtual machines",
			);
		case "aarch64-unknown-linux-gnu":
			return t(
				"devices:setup.platform.hint.linuxArm64",
				"Raspberry Pi 4 and 5, Graviton, Ampere",
			);
		case "x86_64-apple-darwin":
			return t(
				"devices:setup.platform.hint.macIntel",
				"Macs with an Intel processor",
			);
		default:
			return t("devices:setup.platform.hint.macArm", "Macs with M1 or later");
	}
}

function modeHint(t: DevicesT, mode: SetupMode): string {
	switch (mode) {
		case "binary":
			return t(
				"devices:setup.platform.mode.binary",
				"The agent runs as a program on the device. One extra command starts it at boot.",
			);
		case "docker":
			return t(
				"devices:setup.platform.mode.docker",
				"The agent runs in a container from the pinned image. Needs Docker with Compose.",
			);
		default:
			return t(
				"devices:setup.platform.mode.both",
				"The package holds both. You pick one on the device.",
			);
	}
}

function targetCard(
	t: DevicesT,
	option: TargetOption,
): ChoiceOption<ReleaseTarget> {
	const label = enumLabel(t, "target", option.target);
	return {
		value: option.target,
		title: label,
		icon: TARGET_ICON[option.target],
		disabled: !option.available,
		hint: (
			<>
				<span className="block">{targetHint(t, option.target)}</span>
				{option.size === undefined ? null : (
					<span className="block font-mono font-medium text-ink-2 tabular-nums">
						{t("devices:setup.platform.size", "Agent {{size}}", {
							size: megabytes(option.size),
						})}
					</span>
				)}
				{option.available && option.deferredDownload ? (
					<FieldNote tone="info" icon={Info}>
						{t(
							"devices:setup.platform.deferred",
							"Downloads on first start (larger than 256 MiB). Needs internet, curl and shasum on the device.",
						)}
					</FieldNote>
				) : null}
				{option.available ? null : (
					<GateInline kind="unsupported" className="mt-0.5">
						{t(
							"devices:setup.platform.noPackage",
							"This release has no package for {{platform}}.",
							{ platform: label },
						)}
					</GateInline>
				)}
			</>
		),
	};
}

function modeCard(
	t: DevicesT,
	mode: SetupMode,
	option: TargetOption | undefined,
): ChoiceOption<SetupMode> {
	const available = !option || modeAvailable(option, mode);
	return {
		value: mode,
		title: enumLabel(t, "packageMode", mode),
		icon: MODE_ICON[mode],
		disabled: !available,
		hint: (
			<>
				<span className="block">{modeHint(t, mode)}</span>
				{available ? null : (
					<GateInline kind="platform" className="mt-0.5">
						{option?.mac
							? t(
									"devices:setup.platform.dockerLinuxOnly",
									"Docker packages exist only for Linux.",
								)
							: t(
									"devices:setup.platform.noMode",
									"This release has no package for this mode.",
								)}
					</GateInline>
				)}
			</>
		),
	};
}

/** `name@sha256:` plus the first digits of the digest; the full reference is in the hover. */
function shortImage(image: string): string {
	const [name, digest = ""] = image.split("@");
	return `${name}@${digest.slice(0, 15)}…`;
}

const CARD_GRID = "[&_[role=radiogroup]]:grid-cols-1";

/** Step 2: the computer that runs the agent, and how it runs it. */
export function PlatformStep({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const { draft, update, tried, options, option, release, goTo } = useSetup();
	const [switched, setSwitched] = useState<SetupMode>();
	const image = release?.manifest.container?.image;
	const modeOk = modeAvailable(option, draft.mode);

	const header = (
		<WizardStepHeader
			headingRef={headingRef}
			step={3}
			total={STEP_COUNT}
			title={t("setup.platform.title", "Choose the platform")}
			lede={t(
				"setup.platform.lede",
				"Pick the computer that will run the agent. The package only works on that kind of computer.",
			)}
		/>
	);
	if (!release)
		return (
			<>
				{header}
				<StateView
					kind="notloaded"
					title={t(
						"setup.platform.noRelease",
						"The agent release isn't verified yet",
					)}
					text={t(
						"setup.platform.noReleaseText",
						"The platforms come from the signed release. Go back to Check to verify it.",
					)}
					actions={
						<DvButton size="sm" onClick={() => goTo(0)}>
							{t("setup.platform.toCheck", "Go to Check")}
						</DvButton>
					}
				/>
			</>
		);

	const pickTarget = (target: ReleaseTarget) => {
		const next = options.find((item) => item.target === target);
		if (!next?.available) return;
		const keep = modeAvailable(next, draft.mode);
		const mode = keep ? draft.mode : defaultMode(next);
		setSwitched(draft.mode && !keep ? mode : undefined);
		update({ target, mode, startMode: undefined });
	};

	return (
		<>
			{header}
			<div className="flex flex-col gap-1.5">
				<ChoiceCards
					id="dv-setup-platform"
					legend={t("setup.platform.legend", "What will run the agent?")}
					value={draft.target}
					onValueChange={pickTarget}
					options={options.map((item) => targetCard(t, item))}
					className={`${CARD_GRID} @[560px]/setup:[&_[role=radiogroup]]:grid-cols-2`}
				/>
				{tried && !draft.target ? (
					<FieldNote tone="critical" icon={OctagonX}>
						{t(
							"setup.platform.error.target",
							"Pick the platform the device runs on.",
						)}
					</FieldNote>
				) : null}
			</div>
			<div className="flex flex-col gap-1.5">
				<p className="text-xs text-muted-foreground">
					<Trans
						t={t}
						i18nKey="setup.platform.unsure"
						defaults="Not sure? Run this on the device. <1>Linux aarch64</1> means Linux (ARM 64-bit); <1>Darwin arm64</1> means Mac (Apple silicon)."
						components={{ 1: <Mono /> }}
					/>
				</p>
				<CommandBlock command="uname -sm" />
			</div>
			<div className="flex flex-col gap-1.5">
				<ChoiceCards
					id="dv-setup-mode"
					legend={t("setup.platform.modeLegend", "Run with")}
					value={modeOk ? draft.mode : undefined}
					onValueChange={(mode) => {
						setSwitched(undefined);
						update({ mode, startMode: undefined });
					}}
					options={MODES.map((mode) => modeCard(t, mode, option))}
					className={`${CARD_GRID} @[560px]/setup:[&_[role=radiogroup]]:grid-cols-3`}
				/>
				{tried && draft.target && !modeOk ? (
					<FieldNote tone="critical" icon={OctagonX}>
						{t(
							"setup.platform.error.mode",
							"Pick how the device runs the agent.",
						)}
					</FieldNote>
				) : null}
				{switched ? (
					<FieldNote tone="info" icon={Info}>
						{t(
							"setup.platform.switched",
							"Switched to {{mode}}: the mode you had isn't available for this platform.",
							{ mode: enumLabel(t, "packageMode", switched) },
						)}
					</FieldNote>
				) : null}
				{image ? (
					<p className="text-xs text-muted-foreground">
						<Trans
							t={t}
							i18nKey="setup.platform.image"
							defaults="Docker image <1/> · pinned by digest in the signed release."
							components={{
								1: (
									<span className="font-mono" title={image}>
										{shortImage(image)}
									</span>
								),
							}}
						/>
					</p>
				) : null}
			</div>
		</>
	);
}
