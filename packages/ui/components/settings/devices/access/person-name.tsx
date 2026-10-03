"use client";

import { useTranslation } from "@flow-like/locales";
import type { UserIdentity } from "../../../../hooks/use-user-lookup";
import { cx } from "../primitives/tone";

/**
 * The display name of a looked-up account; undefined while the directory has
 * none (the lookup runs or failed, or the account has no name yet). The lookup
 * falls back to the account id, which is never a label here (R3): callers
 * show a neutral phrase instead.
 */
export function identityName(
	identity: Pick<UserIdentity, "isResolved" | "label" | "accountId">,
	userId?: string | null,
): string | undefined {
	if (!identity.isResolved) return undefined;
	const { label } = identity;
	return label && label !== identity.accountId && label !== userId
		? label
		: undefined;
}

/** A person without a name to show: "Unknown account", with the account id as a second, technical line when asked. */
export function UnknownPerson({
	userId,
	pending = false,
	unreachable = false,
	className,
}: Readonly<{
	/** Shown in mono under the phrase: the only way left to tell who it is. */
	userId?: string;
	pending?: boolean;
	/** The directory gave no answer for this account. */
	unreachable?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<span
			data-person-unknown=""
			className={cx("inline-flex min-w-0 flex-col", className)}
		>
			<span
				className="inline-flex min-w-0 items-center gap-1.5"
				{...(unreachable
					? {
							title: t(
								"access.person.unknownTitle",
								"This account couldn't be looked up from here",
							),
						}
					: {})}
			>
				<span
					aria-hidden
					className="inline-flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-[10px] font-semibold text-ink-2"
				>
					?
				</span>
				<span className="truncate">
					{pending
						? t("access.person.lookingUp", "Looking up…")
						: t("access.person.unknown", "Unknown account")}
				</span>
			</span>
			{userId ? (
				<span className="mt-0.5 truncate font-mono text-xs text-muted-foreground">
					{userId}
				</span>
			) : null}
		</span>
	);
}
