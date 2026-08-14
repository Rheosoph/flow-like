"use client";

import { useTranslation } from "@flow-like/locales";
import * as React from "react";

import type { DropdownMenuProps } from "@radix-ui/react-dropdown-menu";

import {
	KeyboardIcon,
	MoreHorizontalIcon,
	SubscriptIcon,
	SuperscriptIcon,
} from "lucide-react";
import { KEYS } from "platejs";
import { useEditorRef } from "platejs/react";

import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../..";

import { ToolbarButton } from "./toolbar";

export function MoreToolbarButton(props: DropdownMenuProps) {
	const { t } = useTranslation("common");
	const editor = useEditorRef();
	const [open, setOpen] = React.useState(false);

	return (
		<DropdownMenu open={open} onOpenChange={setOpen} modal={false} {...props}>
			<DropdownMenuTrigger asChild>
				<ToolbarButton pressed={open} tooltip="Insert">
					<MoreHorizontalIcon />
				</ToolbarButton>
			</DropdownMenuTrigger>

			<DropdownMenuContent
				className="ignore-click-outside/toolbar flex max-h-[500px] min-w-[180px] flex-col overflow-y-auto"
				align="start"
			>
				<DropdownMenuGroup>
					<DropdownMenuItem
						onSelect={() => {
							editor.tf.toggleMark(KEYS.kbd);
							editor.tf.collapse({ edge: "end" });
							editor.tf.focus();
						}}
					>
						<KeyboardIcon />
						{t('keyboardInput', 'Keyboard input')}
					</DropdownMenuItem>

					<DropdownMenuItem
						onSelect={() => {
							editor.tf.toggleMark(KEYS.sup, {
								remove: KEYS.sub,
							});
							editor.tf.focus();
						}}
					>
						<SuperscriptIcon />
						{t('superscript', 'Superscript')}
						{/* (⌘+,) */}
					</DropdownMenuItem>
					<DropdownMenuItem
						onSelect={() => {
							editor.tf.toggleMark(KEYS.sub, {
								remove: KEYS.sup,
							});
							editor.tf.focus();
						}}
					>
						<SubscriptIcon />
						{t('subscript', 'Subscript')}
						{/* (⌘+.) */}
					</DropdownMenuItem>
				</DropdownMenuGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
