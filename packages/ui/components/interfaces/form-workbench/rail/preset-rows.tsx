"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, X } from "lucide-react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { Preset } from "../contracts";

const ROW =
	"w-full rounded-md px-2 text-left hover:bg-row-hover focus-visible:bg-row-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring";

function RowTitle({
	preset,
	touch,
}: Readonly<{ preset: Preset; touch: boolean }>) {
	const { t } = useTranslation("interfaces");
	return (
		<span className="flex items-baseline gap-2">
			<span className="min-w-0 flex-1 truncate text-ui font-medium">
				{preset.name}
			</span>
			{preset.openDefault ? (
				<span className="flex-none text-xs text-muted-foreground">
					{t("workbench.preset.menu.onOpen", "On open")}
				</span>
			) : null}
			{!touch && preset.digit > 0 ? (
				<span className="w-4 flex-none text-right font-mono text-xs text-muted-foreground group-hover:invisible">
					{preset.digit}
				</span>
			) : null}
		</span>
	);
}

/** The cross takes the digit's place on hover (a mouse); it is always there on a touch screen. */
function DeleteButton({
	name,
	touch,
	onDelete,
}: Readonly<{ name: string; touch: boolean; onDelete: () => void }>) {
	const { t } = useTranslation("interfaces");
	return (
		<button
			type="button"
			tabIndex={-1}
			aria-label={t("workbench.preset.menu.delete", "Delete {{name}}", {
				name,
			})}
			onClick={onDelete}
			className={cx(
				"absolute flex items-center justify-center rounded-md text-muted-foreground hover:bg-row-hover hover:text-foreground",
				touch
					? "top-1/2 right-1 size-11 -translate-y-1/2"
					: "invisible top-2 right-2 size-6 group-hover:visible",
			)}
		>
			<X aria-hidden className="size-3.5" />
		</button>
	);
}

export interface PresetRowProps {
	readonly preset: Preset;
	readonly summary: string;
	readonly active: boolean;
	readonly touch: boolean;
	readonly onApply: () => void;
	readonly onDelete: () => void;
}

/** One preset: check, name, "On open", digit, and its summary line (spec S1). */
export function PresetRow(props: Readonly<PresetRowProps>) {
	const { preset, summary, active, touch, onApply, onDelete } = props;
	return (
		<li className="group relative">
			<button
				type="button"
				data-preset-item=""
				data-preset-id={preset.id}
				aria-current={active ? "true" : undefined}
				onClick={onApply}
				className={cx(
					ROW,
					"flex items-start gap-2",
					touch ? "min-h-14 py-2 pr-12" : "min-h-11 py-1.5",
				)}
			>
				<span className="mt-0.5 flex w-4 flex-none justify-center text-foreground">
					{active ? <Check aria-hidden className="size-3.25" /> : null}
				</span>
				<span className="min-w-0 flex-1">
					<RowTitle preset={preset} touch={touch} />
					<span className="block truncate text-xs/4 text-muted-foreground">
						{summary}
					</span>
				</span>
			</button>
			<DeleteButton name={preset.name} touch={touch} onDelete={onDelete} />
		</li>
	);
}

export interface ActionRowProps {
	readonly label: string;
	readonly hint?: string;
	readonly touch: boolean;
	readonly onSelect: () => void;
}

/** An action under the presets: 32 px with a mono hint (a count of changes, a key). */
export function ActionRow({
	label,
	hint,
	touch,
	onSelect,
}: Readonly<ActionRowProps>) {
	return (
		<button
			type="button"
			data-preset-item=""
			data-preset-action=""
			onClick={onSelect}
			className={cx(
				ROW,
				"flex items-center gap-3 text-[12.5px]",
				touch ? "min-h-14 text-sm" : "h-8",
			)}
		>
			<span className="min-w-0 flex-1 truncate">{label}</span>
			{hint ? (
				<span className="flex-none font-mono text-xs text-muted-foreground">
					{hint}
				</span>
			) : null}
		</button>
	);
}
