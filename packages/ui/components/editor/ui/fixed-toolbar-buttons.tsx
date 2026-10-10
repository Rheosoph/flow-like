"use client";

import {
	ArrowUpToLineIcon,
	BaselineIcon,
	BoldIcon,
	Code2Icon,
	HighlighterIcon,
	ItalicIcon,
	PaintBucketIcon,
	StrikethroughIcon,
	UnderlineIcon,
	WandSparklesIcon,
} from "lucide-react";
import { KEYS } from "platejs";
import { useEditorReadOnly } from "platejs/react";

import { AIToolbarButton } from "./ai-toolbar-button";
import { AlignToolbarButton } from "./align-toolbar-button";
import { CommentToolbarButton } from "./comment-toolbar-button";
import { EmojiToolbarButton } from "./emoji-toolbar-button";
import { ExportToolbarButton } from "./export-toolbar-button";
import { FontColorToolbarButton } from "./font-color-toolbar-button";
import { FontSizeToolbarButton } from "./font-size-toolbar-button";
import { RedoToolbarButton, UndoToolbarButton } from "./history-toolbar-button";
import { ImportToolbarButton } from "./import-toolbar-button";
import {
	IndentToolbarButton,
	OutdentToolbarButton,
} from "./indent-toolbar-button";
import { InsertToolbarButton } from "./insert-toolbar-button";
import { LineHeightToolbarButton } from "./line-height-toolbar-button";
import { LinkToolbarButton } from "./link-toolbar-button";
import {
	BulletedListToolbarButton,
	NumberedListToolbarButton,
	TodoListToolbarButton,
} from "./list-toolbar-button";
import { MarkToolbarButton } from "./mark-toolbar-button";
import { MediaToolbarButton } from "./media-toolbar-button";
import { ModeToolbarButton } from "./mode-toolbar-button";
import { MoreToolbarButton } from "./more-toolbar-button";
import { TableToolbarButton } from "./table-toolbar-button";
import { ToggleToolbarButton } from "./toggle-toolbar-button";
import { ToolbarGroup } from "./toolbar";
import { TurnIntoToolbarButton } from "./turn-into-toolbar-button";

export function FixedToolbarButtons() {
	const readOnly = useEditorReadOnly();

	return (
		<div className="flex w-full min-w-0 flex-col">
			{!readOnly && (
				<div
					className="flex min-w-0 flex-wrap items-center gap-y-1 border-b border-border/60 px-1.5 py-1"
					data-toolbar-row="formatting"
				>
					<ToolbarGroup className="shrink-0">
						<UndoToolbarButton />
						<RedoToolbarButton />
					</ToolbarGroup>

					<ToolbarGroup className="shrink-0">
						<TurnIntoToolbarButton />
					</ToolbarGroup>

					<ToolbarGroup className="shrink-0">
						<FontSizeToolbarButton />
					</ToolbarGroup>

					<ToolbarGroup className="shrink-0">
						<MarkToolbarButton nodeType={KEYS.bold} tooltip="Bold (⌘+B)">
							<BoldIcon />
						</MarkToolbarButton>

						<MarkToolbarButton nodeType={KEYS.italic} tooltip="Italic (⌘+I)">
							<ItalicIcon />
						</MarkToolbarButton>

						<MarkToolbarButton
							nodeType={KEYS.underline}
							tooltip="Underline (⌘+U)"
						>
							<UnderlineIcon />
						</MarkToolbarButton>
					</ToolbarGroup>

					<ToolbarGroup className="shrink-0">
						<MarkToolbarButton
							nodeType={KEYS.strikethrough}
							tooltip="Strikethrough (⌘+⇧+M)"
						>
							<StrikethroughIcon />
						</MarkToolbarButton>

						<MarkToolbarButton nodeType={KEYS.code} tooltip="Code (⌘+E)">
							<Code2Icon />
						</MarkToolbarButton>

						<FontColorToolbarButton nodeType={KEYS.color} tooltip="Text color">
							<BaselineIcon />
						</FontColorToolbarButton>

						<FontColorToolbarButton
							nodeType={KEYS.backgroundColor}
							tooltip="Background color"
						>
							<PaintBucketIcon />
						</FontColorToolbarButton>
					</ToolbarGroup>

					<ToolbarGroup className="shrink-0">
						<AlignToolbarButton />

						<NumberedListToolbarButton />
						<BulletedListToolbarButton />
						<TodoListToolbarButton />
						<ToggleToolbarButton />
					</ToolbarGroup>
				</div>
			)}

			<div
				className="flex min-w-0 flex-wrap items-center gap-y-1 px-1.5 py-1"
				data-toolbar-row="actions"
			>
				{!readOnly && (
					<>
						<ToolbarGroup className="shrink-0">
							<AIToolbarButton tooltip="AI commands">
								<WandSparklesIcon />
							</AIToolbarButton>
						</ToolbarGroup>

						<ToolbarGroup className="shrink-0">
							<InsertToolbarButton />
							<LinkToolbarButton />
							<TableToolbarButton />
							<EmojiToolbarButton />
						</ToolbarGroup>

						<ToolbarGroup className="shrink-0">
							<MediaToolbarButton nodeType={KEYS.img} />
							<MediaToolbarButton nodeType={KEYS.video} />
							<MediaToolbarButton nodeType={KEYS.audio} />
							<MediaToolbarButton nodeType={KEYS.file} />
						</ToolbarGroup>

						<ToolbarGroup className="shrink-0">
							<LineHeightToolbarButton />
							<OutdentToolbarButton />
							<IndentToolbarButton />
						</ToolbarGroup>

						<ToolbarGroup className="shrink-0">
							<MoreToolbarButton />
						</ToolbarGroup>
					</>
				)}

				<div className="ml-auto flex flex-wrap items-center gap-y-1">
					{!readOnly && (
						<ToolbarGroup className="shrink-0">
							<ExportToolbarButton>
								<ArrowUpToLineIcon />
							</ExportToolbarButton>
							<ImportToolbarButton />
						</ToolbarGroup>
					)}

					<ToolbarGroup className="shrink-0">
						<MarkToolbarButton nodeType={KEYS.highlight} tooltip="Highlight">
							<HighlighterIcon />
						</MarkToolbarButton>
						<CommentToolbarButton />
					</ToolbarGroup>

					<ToolbarGroup className="shrink-0">
						<ModeToolbarButton />
					</ToolbarGroup>
				</div>
			</div>
		</div>
	);
}
