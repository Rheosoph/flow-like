"use client";

import { useTranslation } from "@flow-like/locales";
import * as React from "react";

import type { TComboboxInputElement, TMentionElement } from "platejs";
import type { PlateElementProps } from "platejs/react";

import { getMentionOnSelectItem } from "@platejs/mention";
import { IS_APPLE, KEYS } from "platejs";
import {
	PlateElement,
	useFocused,
	useReadOnly,
	useSelected,
} from "platejs/react";

import { cn } from "../../../lib/utils";
import { useMounted } from "../hooks/use-mounted";
import { useMentionItems } from "../mention-items-context";

import {
	InlineCombobox,
	InlineComboboxContent,
	InlineComboboxEmpty,
	InlineComboboxGroup,
	InlineComboboxInput,
	InlineComboboxItem,
} from "./inline-combobox";

export function MentionElement(
	props: PlateElementProps<TMentionElement> & {
		prefix?: string;
	},
) {
	const element = props.element;

	const selected = useSelected();
	const focused = useFocused();
	const mounted = useMounted();
	const readOnly = useReadOnly();

	return (
		<PlateElement
			{...props}
			className={cn(
				"inline-block rounded-md bg-muted px-1.5 py-0.5 align-baseline text-sm font-medium",
				!readOnly && "cursor-pointer",
				selected && focused && "ring-2 ring-ring",
				element.children[0][KEYS.bold] === true && "font-bold",
				element.children[0][KEYS.italic] === true && "italic",
				element.children[0][KEYS.underline] === true && "underline",
			)}
			attributes={{
				...props.attributes,
				contentEditable: false,
				"data-slate-value": element.value,
				draggable: true,
			}}
		>
			{mounted && IS_APPLE ? (
				// Mac OS IME https://github.com/ianstormtaylor/slate/issues/3490
				<React.Fragment>
					{props.children}
					{props.prefix}
					{element.value}
				</React.Fragment>
			) : (
				// Others like Android https://github.com/ianstormtaylor/slate/pull/5360
				<React.Fragment>
					{props.prefix}
					{element.value}
					{props.children}
				</React.Fragment>
			)}
		</PlateElement>
	);
}

const onSelectItem = getMentionOnSelectItem();

export function MentionInputElement(
	props: PlateElementProps<TComboboxInputElement>,
) {
	const { t } = useTranslation("common");
	const { editor, element } = props;
	const [search, setSearch] = React.useState("");
	const items = useMentionItems();

	return (
		<PlateElement {...props} as="span">
			<InlineCombobox
				value={search}
				element={element}
				setValue={setSearch}
				showTrigger={false}
				trigger="@"
			>
				<span className="inline-block rounded-md bg-muted px-1.5 py-0.5 align-baseline text-sm ring-ring focus-within:ring-2">
					<InlineComboboxInput />
				</span>

				<InlineComboboxContent className="my-1.5">
					<InlineComboboxEmpty>
						{t("noResults2", "No results")}
					</InlineComboboxEmpty>

					<InlineComboboxGroup>
						{items.map((item) => (
							<InlineComboboxItem
								key={item.key}
								value={item.text}
								onClick={() => {
									if (item.onSelect) {
										item.onSelect(editor, search);
										return;
									}
									onSelectItem(editor, item, search);
								}}
							>
								{item.text}
							</InlineComboboxItem>
						))}
					</InlineComboboxGroup>
				</InlineComboboxContent>
			</InlineCombobox>

			{props.children}
		</PlateElement>
	);
}
