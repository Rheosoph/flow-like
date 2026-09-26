"use client";

import { useTranslation } from "@flow-like/locales";
import { formatDateValue } from "@platejs/date";
import type { TDateElement } from "platejs";
import type { PlateElementProps } from "platejs/react";

import { PlateElement, useReadOnly } from "platejs/react";

import { Calendar } from "../../..";
import { Popover, PopoverContent, PopoverTrigger } from "../../..";
import { cn } from "../../../lib/utils";
import { getDateElementDate, getDateElementLabel } from "./date-label";

export function DateElement(props: PlateElementProps<TDateElement>) {
	const { t } = useTranslation("common");
	const { editor, element } = props;

	const readOnly = useReadOnly();
	const label = getDateElementLabel(element);

	const trigger = (
		<span
			className={cn(
				"w-fit cursor-pointer rounded-sm bg-muted px-1 text-muted-foreground",
			)}
			contentEditable={false}
			draggable
		>
			{label ?? <span>{t("pickADate", "Pick a date")}</span>}
		</span>
	);

	if (readOnly) {
		return trigger;
	}

	return (
		<PlateElement
			{...props}
			className="inline-block"
			attributes={{
				...props.attributes,
				contentEditable: false,
			}}
		>
			<Popover>
				<PopoverTrigger asChild>{trigger}</PopoverTrigger>
				<PopoverContent className="w-auto p-0">
					<Calendar
						selected={getDateElementDate(element)}
						onSelect={(date) => {
							if (!date) return;

							editor.tf.setNodes(
								{ date: formatDateValue(date), rawDate: undefined },
								{ at: element },
							);
						}}
						mode="single"
						initialFocus
					/>
				</PopoverContent>
			</Popover>
			{props.children}
		</PlateElement>
	);
}
