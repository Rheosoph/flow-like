"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowDownLeftIcon,
	ArrowUpRightIcon,
	BrainIcon,
	LockKeyholeIcon,
} from "lucide-react";
import { useId } from "react";
import { Switch } from "../ui/switch";
import type { RoomLimitsDraft } from "./room-limits-model";

export type RoomMemoryDraft = Pick<
	RoomLimitsDraft,
	"useOwnerMemory" | "useSharedMemory" | "shareMemory"
>;

function MemorySwitch({
	label,
	hint,
	checked,
	disabled,
	onChange,
}: Readonly<{
	label: string;
	hint: string;
	checked: boolean;
	disabled?: boolean;
	onChange: (checked: boolean) => void;
}>) {
	const id = useId();
	return (
		<div className="flex items-start gap-4 py-3">
			<div className="min-w-0 flex-1 space-y-1">
				<label
					htmlFor={id}
					className="cursor-pointer text-sm font-medium leading-5"
				>
					{label}
				</label>
				<p
					id={`${id}-hint`}
					className="text-xs leading-relaxed text-muted-foreground"
				>
					{hint}
				</p>
			</div>
			<Switch
				id={id}
				aria-label={label}
				aria-describedby={`${id}-hint`}
				checked={checked}
				disabled={disabled}
				onCheckedChange={onChange}
				className="mt-1"
			/>
		</div>
	);
}

/** These choices apply to the bot's whole group memory, never to individual facts. */
export function RoomMemorySettings({
	value,
	disabled,
	onChange,
}: Readonly<{
	value: RoomMemoryDraft;
	disabled?: boolean;
	onChange: (value: RoomMemoryDraft) => void;
}>) {
	const { t } = useTranslation("bots");
	const sources = [
		t("roomLimits.memory.thisGroup", "This group"),
		...(value.useOwnerMemory
			? [t("roomLimits.memory.privateSource", "Your 1:1")]
			: []),
		...(value.useSharedMemory
			? [t("roomLimits.memory.sharedSource", "Shared groups")]
			: []),
	];
	return (
		<section
			className="rounded-xl border bg-card"
			aria-label={t("roomLimits.memory.title", "Knowledge and memory")}
		>
			<div className="flex items-center gap-2.5 border-b px-4 py-3">
				<span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
					<BrainIcon className="size-4" aria-hidden />
				</span>
				<div>
					<h3 className="text-sm font-semibold">
						{t("roomLimits.memory.title", "Knowledge and memory")}
					</h3>
					<p className="text-xs text-muted-foreground">
						{t(
							"roomLimits.memory.subtitle",
							"Choose what comes in and what can leave.",
						)}
					</p>
				</div>
			</div>
			<div className="px-4 pt-3">
				<p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
					<ArrowDownLeftIcon className="size-3.5" aria-hidden />
					{t("roomLimits.memory.recall", "Use in this group")}
				</p>
				<div className="divide-y">
					<MemorySwitch
						label={t("roomLimits.memory.owner", "Use 1:1 memory")}
						hint={t(
							"roomLimits.memory.ownerHint",
							"Recall saved knowledge from your 1:1 with this bot. It may appear in group answers.",
						)}
						checked={value.useOwnerMemory}
						disabled={disabled}
						onChange={(useOwnerMemory) =>
							onChange({ ...value, useOwnerMemory })
						}
					/>
					<MemorySwitch
						label={t("roomLimits.memory.shared", "Use shared memory")}
						hint={t(
							"roomLimits.memory.sharedHint",
							"Recall knowledge this bot's other groups have chosen to share.",
						)}
						checked={value.useSharedMemory}
						disabled={disabled}
						onChange={(useSharedMemory) =>
							onChange({ ...value, useSharedMemory })
						}
					/>
				</div>
			</div>
			<div className="border-t px-4 pt-3">
				<p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
					<ArrowUpRightIcon className="size-3.5" aria-hidden />
					{t("roomLimits.memory.contribute", "Use beyond this group")}
				</p>
				<MemorySwitch
					label={t("roomLimits.memory.share", "Contribute to shared memory")}
					hint={t(
						"roomLimits.memory.shareHint",
						"Share this group's saved knowledge with your 1:1 and this bot's groups that use shared memory. Turning this off stops future recall elsewhere.",
					)}
					checked={value.shareMemory}
					disabled={disabled}
					onChange={(shareMemory) => onChange({ ...value, shareMemory })}
				/>
			</div>
			<div
				className="space-y-2 rounded-b-xl border-t bg-muted/40 px-4 py-3"
				aria-live="polite"
				aria-atomic="true"
			>
				<div className="flex flex-wrap items-center gap-1.5 text-xs">
					<span className="mr-1 text-muted-foreground">
						{t("roomLimits.memory.recallSummary", "Can recall:")}
					</span>
					{sources.map((source) => (
						<span
							key={source}
							className="rounded-full border bg-background px-2 py-0.5"
						>
							{source}
						</span>
					))}
				</div>
				<p className="flex items-start gap-1.5 text-xs leading-relaxed text-muted-foreground">
					{value.shareMemory ? (
						<ArrowUpRightIcon className="mt-0.5 size-3 shrink-0" aria-hidden />
					) : (
						<LockKeyholeIcon className="mt-0.5 size-3 shrink-0" aria-hidden />
					)}
					{value.shareMemory
						? t(
								"roomLimits.memory.sharingSummary",
								"Saved knowledge from this group is available in your 1:1 and groups that use shared memory.",
							)
						: t(
								"roomLimits.memory.localSummary",
								"What this bot learns here stays in this group.",
							)}
				</p>
			</div>
		</section>
	);
}
