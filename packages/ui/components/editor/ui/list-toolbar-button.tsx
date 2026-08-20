"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import * as React from "react";

import { ListStyleType, someList, toggleList } from "@platejs/list";
import {
	useIndentTodoToolBarButton,
	useIndentTodoToolBarButtonState,
} from "@platejs/list/react";
import { List, ListOrdered, ListTodoIcon } from "lucide-react";
import { useEditorRef, useEditorSelector } from "platejs/react";

import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../..";

import {
	ToolbarButton,
	ToolbarSplitButton,
	ToolbarSplitButtonPrimary,
	ToolbarSplitButtonSecondary,
} from "./toolbar";

export function BulletedListToolbarButton() {
	const editor = useEditorRef();
	const [open, setOpen] = React.useState(false);

	const pressed = useEditorSelector(
		(editor) =>
			someList(editor, [
				ListStyleType.Disc,
				ListStyleType.Circle,
				ListStyleType.Square,
			]),
		[],
	);

	return (
		<ToolbarSplitButton pressed={open}>
			<ToolbarSplitButtonPrimary
				className="data-[state=on]:bg-accent data-[state=on]:text-accent-foreground"
				onClick={() => {
					toggleList(editor, {
						listStyleType: ListStyleType.Disc,
					});
				}}
				data-state={pressed ? "on" : "off"}
			>
				<List className="size-4" />
			</ToolbarSplitButtonPrimary>

			<DropdownMenu open={open} onOpenChange={setOpen} modal={false}>
				<DropdownMenuTrigger asChild>
					<ToolbarSplitButtonSecondary />
				</DropdownMenuTrigger>

				<DropdownMenuContent align="start" alignOffset={-32}>
					<DropdownMenuGroup>
						<DropdownMenuItem
							onClick={() =>
								toggleList(editor, {
									listStyleType: ListStyleType.Disc,
								})
							}
						>
							<div className="flex items-center gap-2">
								<Trans i18nKey="divClassnamesize2RoundedfullBorderBordercurrentBgcurrentDefault">
									<div className="size-2 rounded-full border border-current bg-current" />
									Default
								</Trans>
							</div>
						</DropdownMenuItem>
						<DropdownMenuItem
							onClick={() =>
								toggleList(editor, {
									listStyleType: ListStyleType.Circle,
								})
							}
						>
							<div className="flex items-center gap-2">
								<Trans i18nKey="divClassnamesize2RoundedfullBorderBordercurrentCircle">
									<div className="size-2 rounded-full border border-current" />
									Circle
								</Trans>
							</div>
						</DropdownMenuItem>
						<DropdownMenuItem
							onClick={() =>
								toggleList(editor, {
									listStyleType: ListStyleType.Square,
								})
							}
						>
							<div className="flex items-center gap-2">
								<Trans i18nKey="divClassnamesize2BorderBordercurrentBgcurrentSquare">
									<div className="size-2 border border-current bg-current" />
									Square
								</Trans>
							</div>
						</DropdownMenuItem>
					</DropdownMenuGroup>
				</DropdownMenuContent>
			</DropdownMenu>
		</ToolbarSplitButton>
	);
}

export function NumberedListToolbarButton() {
	const { t } = useTranslation("common");
	const editor = useEditorRef();
	const [open, setOpen] = React.useState(false);

	const pressed = useEditorSelector(
		(editor) =>
			someList(editor, [
				ListStyleType.Decimal,
				ListStyleType.LowerAlpha,
				ListStyleType.UpperAlpha,
				ListStyleType.LowerRoman,
				ListStyleType.UpperRoman,
			]),
		[],
	);

	return (
		<ToolbarSplitButton pressed={open}>
			<ToolbarSplitButtonPrimary
				className="data-[state=on]:bg-accent data-[state=on]:text-accent-foreground"
				onClick={() =>
					toggleList(editor, {
						listStyleType: ListStyleType.Decimal,
					})
				}
				data-state={pressed ? "on" : "off"}
			>
				<ListOrdered className="size-4" />
			</ToolbarSplitButtonPrimary>

			<DropdownMenu open={open} onOpenChange={setOpen} modal={false}>
				<DropdownMenuTrigger asChild>
					<ToolbarSplitButtonSecondary />
				</DropdownMenuTrigger>

				<DropdownMenuContent align="start" alignOffset={-32}>
					<DropdownMenuGroup>
						<DropdownMenuItem
							onSelect={() =>
								toggleList(editor, {
									listStyleType: ListStyleType.Decimal,
								})
							}
						>
							{t("decimal123", "Decimal (1, 2, 3)")}
						</DropdownMenuItem>
						<DropdownMenuItem
							onSelect={() =>
								toggleList(editor, {
									listStyleType: ListStyleType.LowerAlpha,
								})
							}
						>
							{t("lowerAlphaABC", "Lower Alpha (a, b, c)")}
						</DropdownMenuItem>
						<DropdownMenuItem
							onSelect={() =>
								toggleList(editor, {
									listStyleType: ListStyleType.UpperAlpha,
								})
							}
						>
							{t("upperAlphaABC", "Upper Alpha (A, B, C)")}
						</DropdownMenuItem>
						<DropdownMenuItem
							onSelect={() =>
								toggleList(editor, {
									listStyleType: ListStyleType.LowerRoman,
								})
							}
						>
							{t("lowerRomanIIiIii", "Lower Roman (i, ii, iii)")}
						</DropdownMenuItem>
						<DropdownMenuItem
							onSelect={() =>
								toggleList(editor, {
									listStyleType: ListStyleType.UpperRoman,
								})
							}
						>
							{t("upperRomanIIiIii", "Upper Roman (I, II, III)")}
						</DropdownMenuItem>
					</DropdownMenuGroup>
				</DropdownMenuContent>
			</DropdownMenu>
		</ToolbarSplitButton>
	);
}

export function TodoListToolbarButton(
	props: React.ComponentProps<typeof ToolbarButton>,
) {
	const state = useIndentTodoToolBarButtonState({ nodeType: "todo" });
	const { props: buttonProps } = useIndentTodoToolBarButton(state);

	return (
		<ToolbarButton {...props} {...buttonProps} tooltip="Todo">
			<ListTodoIcon />
		</ToolbarButton>
	);
}
