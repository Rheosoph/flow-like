"use client";

import { useTranslation } from "@flow-like/locales";
import type { SlateElementProps } from "platejs";

import { User } from "lucide-react";
import { SlateElement } from "platejs";

export interface TUserMentionElement {
	type: "user_mention";
	sub: string;
	children: [{ text: "" }];
	[key: string]: unknown;
}

export function UserMentionElementStatic(
	props: SlateElementProps<TUserMentionElement>,
) {
	const { t } = useTranslation("common");
	const { sub } = props.element;

	return (
		<SlateElement
			{...props}
			as="span"
			className="
				inline-flex items-center gap-1
				px-1.5 py-0.5
				-my-0.5 mx-0.5
				align-baseline
				text-xs font-semibold tracking-tight
				text-blue-700 dark:text-blue-300
				bg-gradient-to-r from-blue-100 via-sky-50 to-cyan-100
				dark:from-blue-950/60 dark:via-sky-950/40 dark:to-cyan-950/60
				rounded-full
				border border-blue-200/60 dark:border-blue-700/40
				shadow-sm shadow-blue-200/50 dark:shadow-blue-900/30
				hover:shadow-md hover:shadow-blue-300/50 dark:hover:shadow-blue-800/40
				hover:border-blue-300 dark:hover:border-blue-600
				hover:from-blue-200 hover:via-sky-100 hover:to-cyan-200
				dark:hover:from-blue-900/70 dark:hover:via-sky-900/50 dark:hover:to-cyan-900/70
				hover:scale-[1.02]
				active:scale-[0.98]
				transition-all duration-200 ease-out
				cursor-pointer
				select-none
			"
			attributes={{
				...props.attributes,
				"data-user-mention-sub": sub,
			}}
		>
			<User className="w-3 h-3 shrink-0 opacity-80" />
			<span contentEditable={false} className="leading-none">{`@${sub}`}</span>
			{props.children}
		</SlateElement>
	);
}
