"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Fingerprint, OctagonX, Tag, TriangleAlert, Users } from "lucide-react";
import { type Ref, useState } from "react";
import { Label } from "../../../../ui/label";
import type { DevicesT } from "../../primitives/area-context";
import { DvButton } from "../../primitives/dv-button";
import { DvInput } from "../../primitives/form-fields";
import { PresenceGlyph } from "../../primitives/presence-glyph";
import { WizardStepHeader } from "../../primitives/wizard";
import { useSetup } from "../setup-context";
import { FieldNote, GroupLabel, IconList, Mono } from "../setup-parts";
import {
	NAME_MAX_BYTES,
	type NameIssue,
	STEP_COUNT,
	duplicateName,
	nameIssue,
	suggestName,
	utf8Bytes,
} from "../setup-state";

const NAME_ID = "dv-setup-name";

export function nameIssueText(t: DevicesT, issue: NameIssue): string {
	switch (issue.code) {
		case "empty":
			return t(
				"devices:setup.name.error.empty",
				"Enter a name for the device.",
			);
		case "control":
			return t(
				"devices:setup.name.error.control",
				"Remove line breaks, tabs and other control characters.",
			);
		case "edge_space":
			return t(
				"devices:setup.name.error.edgeSpace",
				"Remove the space at the start or end of the name.",
			);
		default:
			return t(
				"devices:setup.name.error.tooLong",
				"That's {{bytes, number}} bytes. Use at most {{max, number}}.",
				{ bytes: issue.bytes, max: NAME_MAX_BYTES },
			);
	}
}

/** Step 1: the name the hub signs into the package. */
export function NameStep({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const { draft, update, tried, takenNames, limits } = useSetup();
	const [touched, setTouched] = useState(false);
	const { name } = draft;
	const issue = nameIssue(name);
	const error =
		issue && (touched || tried) ? nameIssueText(t, issue) : undefined;
	const duplicate = issue ? undefined : duplicateName(name, takenNames);
	const bytes = utf8Bytes(name);
	const suggestion = suggestName(takenNames);
	const hours = Math.round(limits.lifetimeS / 3600);

	return (
		<>
			<WizardStepHeader
				headingRef={headingRef}
				step={2}
				total={STEP_COUNT}
				title={t("setup.name.title", "Name the device")}
				lede={t(
					"setup.name.lede",
					"Up to {{max, number}} bytes, no spaces at the start or end.",
					{ max: NAME_MAX_BYTES },
				)}
			/>
			<div
				data-field=""
				data-invalid={error ? "true" : undefined}
				className="flex min-w-0 flex-col gap-1.5"
			>
				<Label htmlFor={NAME_ID} className="text-[13px]/[18px] font-medium">
					{t("setup.name.label", "Name this device")}
				</Label>
				<DvInput
					id={NAME_ID}
					mono
					value={name}
					placeholder={suggestion?.name}
					autoComplete="off"
					autoCapitalize="off"
					spellCheck={false}
					aria-invalid={error ? true : undefined}
					aria-describedby={`${NAME_ID}-hint ${NAME_ID}-msg`}
					onChange={(event) => update({ name: event.target.value })}
					onBlur={() => setTouched(true)}
				/>
				<div className="flex items-start justify-between gap-3">
					<p id={`${NAME_ID}-hint`} className="text-xs text-muted-foreground">
						{t(
							"setup.name.hint",
							"It's signed into the package and stays the device's permanent name.",
						)}
					</p>
					<span className="shrink-0 text-xs whitespace-nowrap text-muted-foreground tabular-nums">
						{bytes === name.length
							? t(
									"setup.name.count",
									"{{bytes, number}} of {{max, number}} bytes",
									{ bytes, max: NAME_MAX_BYTES },
								)
							: t(
									"setup.name.countWide",
									"{{bytes, number}} of {{max, number}} bytes · some letters use 2–4 bytes",
									{ bytes, max: NAME_MAX_BYTES },
								)}
					</span>
				</div>
				<div id={`${NAME_ID}-msg`} aria-live="polite" className="empty:hidden">
					{error ? (
						<FieldNote tone="critical" icon={OctagonX}>
							{error}
						</FieldNote>
					) : duplicate ? (
						<FieldNote tone="warning" icon={TriangleAlert}>
							{t(
								"setup.name.duplicate",
								"You already have a device or setup called {{name}}. Pick another name so you can tell them apart.",
								{ name: duplicate },
							)}
						</FieldNote>
					) : null}
				</div>
				{suggestion && suggestion.name !== name ? (
					<p className="text-xs text-muted-foreground">
						<Trans
							t={t}
							i18nKey="setup.name.suggested"
							defaults="Suggested: <1/> · it follows <2/>."
							components={{
								1: (
									<DvButton
										variant="link"
										size="xs"
										className="font-mono"
										onClick={() => {
											setTouched(true);
											update({ name: suggestion.name });
										}}
									>
										{suggestion.name}
									</DvButton>
								),
								2: <Mono>{suggestion.follows}</Mono>,
							}}
						/>
					</p>
				) : null}
			</div>
			<div className="flex flex-col gap-1.5">
				<GroupLabel>
					{t("setup.name.preview", "In your device list")}
				</GroupLabel>
				<div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1.5 rounded-lg border border-dashed border-border-strong bg-surface-sunken px-3 py-2.5 text-ui">
					<PresenceGlyph kind="pending" decorative />
					<span
						className={
							name
								? "max-w-full truncate font-mono font-medium"
								: "max-w-full truncate font-mono text-muted-foreground"
						}
					>
						{name ||
							suggestion?.name ||
							t("setup.name.placeholder", "device-name")}
					</span>
					<span className="rounded-sm border border-dashed border-border-strong px-1 font-mono text-[11px]/[14px] font-medium text-muted-foreground">
						{t("setup.name.previewTag", "package")}
					</span>
					<span className="text-muted-foreground">
						{t(
							"setup.name.previewState",
							"Waiting · expires in {{count, number}} h",
							{ count: hours },
						)}
					</span>
				</div>
			</div>
			<IconList
				rows={[
					{
						id: "shared",
						icon: Users,
						text: t(
							"setup.name.tip.shared",
							"People you share the device with see this name too.",
						),
					},
					{
						id: "style",
						icon: Tag,
						text: (
							<Trans
								t={t}
								i18nKey="setup.name.tip.style"
								defaults="Name the place, machine or job, like <1>factory-line-4</1> or <1>lab-gpu-03</1>. Letters, digits and dashes read best in commands."
								components={{ 1: <Mono /> }}
							/>
						),
					},
					{
						id: "id",
						icon: Fingerprint,
						text: t(
							"setup.name.tip.id",
							"The device also gets a permanent ID when it's registered. The ID, not the name, is what the hub trusts.",
						),
					},
				]}
			/>
		</>
	);
}
