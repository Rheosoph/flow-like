import type { ReactNode } from "react";
import {
	BOT_CAPABILITY_ICONS,
	type BotCapability,
	capabilityLabel,
} from "./bot-docs-data";
import "./bots.css";

export type Tone = "good" | "warn" | "bad";

export function Segment<T extends string>({
	label,
	value,
	options,
	onChange,
}: {
	label: string;
	value: T;
	options: readonly { value: T; label: string }[];
	onChange: (value: T) => void;
}) {
	return (
		<fieldset className="bd-segment" aria-label={label}>
			{options.map((option) => (
				<button
					key={option.value}
					type="button"
					aria-pressed={option.value === value}
					onClick={() => onChange(option.value)}
				>
					{option.label}
				</button>
			))}
		</fieldset>
	);
}

export function CapabilityChip({
	id,
	tone,
	off = false,
}: {
	id: BotCapability;
	tone?: "accent" | Tone;
	off?: boolean;
}) {
	const Icon = BOT_CAPABILITY_ICONS[id];
	const classes = [
		"bd-chip",
		tone ? `bd-chip--${tone}` : "",
		off ? "bd-chip--off" : "",
	]
		.filter(Boolean)
		.join(" ");
	return (
		<span className={classes}>
			<Icon aria-hidden="true" />
			{capabilityLabel(id)}
		</span>
	);
}

const TONE_PATHS: Record<Tone, ReactNode> = {
	good: <path d="M20 6 9 17l-5-5" />,
	warn: (
		<>
			<path d="M12 9v4" />
			<path d="M12 17h.01" />
			<path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
		</>
	),
	bad: (
		<>
			<circle cx="12" cy="12" r="9" />
			<path d="m15 9-6 6M9 9l6 6" />
		</>
	),
};

export function ToneIcon({ tone }: { tone: Tone }) {
	return (
		<svg
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			strokeWidth="2"
			strokeLinecap="round"
			strokeLinejoin="round"
			aria-hidden="true"
		>
			{TONE_PATHS[tone]}
		</svg>
	);
}

export function Verdict({
	tone,
	title,
	children,
}: {
	tone: Tone;
	title: string;
	children?: ReactNode;
}) {
	return (
		<output className={`bd-verdict bd-verdict--${tone}`} aria-live="polite">
			<ToneIcon tone={tone} />
			<span>
				<strong>{title}</strong>
				{children ? (
					<span className="bd-verdict-detail">{children}</span>
				) : null}
			</span>
		</output>
	);
}

export function Toggle({
	label,
	hint,
	checked,
	disabled = false,
	onChange,
}: {
	label: string;
	hint?: string;
	checked: boolean;
	disabled?: boolean;
	onChange: (checked: boolean) => void;
}) {
	return (
		<label className="bd-toggle" data-disabled={disabled}>
			<input
				type="checkbox"
				checked={checked}
				disabled={disabled}
				onChange={(event) => onChange(event.target.checked)}
			/>
			<span className="bd-toggle-text">
				{label}
				{hint ? <small>{hint}</small> : null}
			</span>
		</label>
	);
}
