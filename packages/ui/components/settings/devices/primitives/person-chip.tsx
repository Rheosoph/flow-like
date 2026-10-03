"use client";

import { useTranslation } from "@flow-like/locales";
import { Avatar, AvatarFallback, AvatarImage } from "../../../ui/avatar";
import { cx } from "./tone";

/** Up to two initials from a display name ("Mira Novak" → "MN"); whole characters, never half a surrogate pair. */
export function initialsOf(name: string): string {
	const parts = name
		.trim()
		.split(/[\s._@-]+/)
		.filter(Boolean)
		.map((part) => Array.from(part));
	const first = parts[0];
	if (!first) return "?";
	const last = parts[parts.length - 1];
	const letters = parts.length > 1 ? [first[0], last[0]] : first.slice(0, 2);
	return letters.join("").toUpperCase();
}

/** SPEC §4.37: 20 px initials avatar + name; "You" for the viewer; the hover shows the email. */
export function PersonChip({
	name,
	email,
	avatarUrl,
	you = false,
	fullName = false,
	className,
}: Readonly<{
	name: string;
	email?: string;
	avatarUrl?: string;
	/** The viewer: rendered as "You" unless `fullName`. */
	you?: boolean;
	fullName?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const shown = you && !fullName ? t("view.person.you", "You") : name;
	const title = email
		? t("view.person.title", "{{name}} · {{email}}", { name, email })
		: name;
	return (
		<span
			data-person=""
			title={title}
			className={cx(
				"inline-flex max-w-full min-w-0 items-center gap-1.5 align-middle",
				className,
			)}
		>
			<Avatar aria-hidden className="size-5">
				{avatarUrl ? <AvatarImage src={avatarUrl} alt="" /> : null}
				<AvatarFallback className="bg-muted text-[10px] leading-none font-semibold text-ink-2">
					{initialsOf(name)}
				</AvatarFallback>
			</Avatar>
			<span className="truncate">{shown}</span>
		</span>
	);
}
