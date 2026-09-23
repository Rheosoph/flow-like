"use client";

import { useTranslation } from "@flow-like/locales";
import type { TDateElement } from "platejs";
import type { SlateElementProps } from "platejs/static";

import { SlateElement } from "platejs/static";

import { getDateElementLabel } from "./date-label";

export function DateElementStatic(props: SlateElementProps<TDateElement>) {
	const { t } = useTranslation("common");
	const label = getDateElementLabel(props.element);

	return (
		<SlateElement className="inline-block" {...props}>
			<span className="w-fit rounded-sm bg-muted px-1 text-muted-foreground">
				{label ?? <span>{t("pickADate", "Pick a date")}</span>}
			</span>
			{props.children}
		</SlateElement>
	);
}
