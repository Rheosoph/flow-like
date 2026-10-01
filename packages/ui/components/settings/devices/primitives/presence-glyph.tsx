"use client";

import { useTranslation } from "@flow-like/locales";
import { cx } from "./tone";

export type PresenceGlyphKind =
	| "online"
	| "late"
	| "offline"
	| "never"
	| "revoked"
	| "pending";

const COLOR: Record<PresenceGlyphKind, string> = {
	online: "text-good-solid",
	late: "text-warning-solid",
	offline: "text-critical-solid",
	never: "text-unknown",
	revoked: "text-unknown",
	pending: "text-unknown",
};

function Shape({ kind }: Readonly<{ kind: PresenceGlyphKind }>) {
	switch (kind) {
		case "online":
			return <circle cx="6" cy="6" r="4.5" fill="currentColor" />;
		case "late":
			return (
				<>
					<circle
						cx="6"
						cy="6"
						r="4.25"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.5"
					/>
					<path d="M6 1.75a4.25 4.25 0 0 1 0 8.5z" fill="currentColor" />
				</>
			);
		case "offline":
			return (
				<>
					<circle
						cx="6"
						cy="6"
						r="4.25"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.5"
					/>
					<path d="M3 9 9 3" stroke="currentColor" strokeWidth="1.5" />
				</>
			);
		case "revoked":
			return (
				<>
					<rect
						x="1.75"
						y="1.75"
						width="8.5"
						height="8.5"
						rx="1.5"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.5"
					/>
					<path
						d="M4.2 4.2l3.6 3.6M7.8 4.2 4.2 7.8"
						stroke="currentColor"
						strokeWidth="1.4"
					/>
				</>
			);
		case "pending":
			return (
				<rect
					x="1.75"
					y="1.75"
					width="8.5"
					height="8.5"
					rx="1.5"
					fill="none"
					stroke="currentColor"
					strokeWidth="1.5"
					strokeDasharray="2 1.6"
				/>
			);
		default:
			return (
				<circle
					cx="6"
					cy="6"
					r="4.25"
					fill="none"
					stroke="currentColor"
					strokeWidth="1.5"
					strokeDasharray="2 1.7"
				/>
			);
	}
}

/** 12 px presence shape (SPEC §4.2): a shape per state, never colour alone. */
export function PresenceGlyph({
	kind,
	label,
	decorative = false,
	className,
}: Readonly<{
	kind: PresenceGlyphKind;
	label?: string;
	/** Next to text that already names the state: hidden from assistive tech. */
	decorative?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const fallback: Record<PresenceGlyphKind, string> = {
		online: t("enum.presence.online", "Online"),
		late: t("enum.presence.late", "Late"),
		offline: t("enum.presence.offline", "Offline"),
		never: t("enum.presence.never", "Never checked in"),
		revoked: t("enum.deviceStatus.revoked", "Revoked"),
		pending: t("enum.presence.pending", "Setup package"),
	};
	return (
		<span
			role={decorative ? undefined : "img"}
			aria-hidden={decorative || undefined}
			aria-label={decorative ? undefined : (label ?? fallback[kind])}
			data-presence={kind}
			className={cx("inline-flex size-3 shrink-0", COLOR[kind], className)}
		>
			<svg viewBox="0 0 12 12" aria-hidden="true" className="block size-full">
				<Shape kind={kind} />
			</svg>
		</span>
	);
}
