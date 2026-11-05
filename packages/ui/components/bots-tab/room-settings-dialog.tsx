"use client";

import { useTranslation } from "@flow-like/locales";
import { ImagePlusIcon, Trash2Icon } from "lucide-react";
import { useId, useRef, useState } from "react";
import {
	useRenameConversation,
	useSetConversationImage,
} from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { RoomAvatar } from "./room-avatar";
import {
	ROOM_IMAGE_ACCEPT,
	RoomImageError,
	type RoomImageProblem,
	prepareRoomImage,
} from "./room-image";

/** A picked picture waiting for Save: a data URL, `null` to remove it, `undefined` to keep it. */
type StagedImage = string | null | undefined;

function useImageProblemText() {
	const { t } = useTranslation("bots");
	return (problem: RoomImageProblem) => {
		switch (problem) {
			case "format":
				return t(
					"crew.roomSettings.imageFormat",
					"Choose a PNG, JPEG, WebP or GIF image.",
				);
			case "size":
				return t(
					"crew.roomSettings.imageSize",
					"Choose an image smaller than 20 MB.",
				);
			default:
				return t(
					"crew.roomSettings.imageUnreadable",
					"This image couldn't be read. Choose another one.",
				);
		}
	};
}

function PicturePicker({
	conversation,
	title,
	preview,
	disabled,
	onPick,
	onRemove,
}: Readonly<{
	conversation: IConversation;
	title: string;
	preview: string | null;
	disabled: boolean;
	onPick: (file: File) => void;
	onRemove: () => void;
}>) {
	const { t } = useTranslation("bots");
	const input = useRef<HTMLInputElement>(null);
	const choose = () => input.current?.click();
	return (
		<div className="flex items-center gap-4">
			<button
				type="button"
				onClick={choose}
				disabled={disabled}
				className="rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
				aria-label={t("crew.roomSettings.upload", "Upload picture")}
			>
				<RoomAvatar
					roomId={conversation.id}
					title={title}
					principalIds={[]}
					imageUrl={preview}
					size={72}
				/>
			</button>
			<div className="flex min-w-0 flex-col items-start gap-1.5">
				<div className="flex flex-wrap gap-1.5">
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={disabled}
						onClick={choose}
					>
						<ImagePlusIcon aria-hidden />
						{preview
							? t("crew.roomSettings.change", "Change picture")
							: t("crew.roomSettings.upload", "Upload picture")}
					</Button>
					{preview && (
						<Button
							type="button"
							variant="ghost"
							size="sm"
							disabled={disabled}
							onClick={onRemove}
						>
							<Trash2Icon aria-hidden />
							{t("crew.roomSettings.remove", "Remove")}
						</Button>
					)}
				</div>
				<p className="text-xs text-muted-foreground">
					{t(
						"crew.roomSettings.imageHint",
						"PNG, JPEG, WebP or GIF, cropped to a square.",
					)}
				</p>
			</div>
			<input
				ref={input}
				type="file"
				accept={ROOM_IMAGE_ACCEPT}
				className="hidden"
				onChange={(event) => {
					const file = event.target.files?.[0];
					event.target.value = "";
					if (file) onPick(file);
				}}
			/>
		</div>
	);
}

function RoomSettingsForm({
	conversation,
	defaultTitle,
	onDone,
}: Readonly<{
	conversation: IConversation;
	defaultTitle?: string;
	onDone: () => void;
}>) {
	const { t } = useTranslation("bots");
	const nameId = useId();
	const rename = useRenameConversation();
	const setImage = useSetConversationImage();
	const problemText = useImageProblemText();
	const [title, setTitle] = useState(conversation.title ?? "");
	const [staged, setStaged] = useState<StagedImage>(undefined);
	const [preparing, setPreparing] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const preview =
		staged === undefined ? (conversation.image_url ?? null) : staged;
	const titleChanged = title.trim() !== (conversation.title ?? "");
	const imageChanged =
		staged !== undefined && staged !== (conversation.image_url ?? null);
	const saving = rename.isPending || setImage.isPending;

	const pick = (file: File) => {
		setError(null);
		setPreparing(true);
		prepareRoomImage(file)
			.then(setStaged)
			.catch((cause: unknown) =>
				setError(
					problemText(
						cause instanceof RoomImageError ? cause.problem : "unreadable",
					),
				),
			)
			.finally(() => setPreparing(false));
	};

	const save = async () => {
		setError(null);
		try {
			if (titleChanged)
				await rename.mutateAsync({
					conversationId: conversation.id,
					title: title.trim(),
				});
			if (imageChanged)
				await setImage.mutateAsync({
					conversationId: conversation.id,
					image: staged ?? null,
				});
			onDone();
		} catch (cause) {
			setError(
				apiErrorMessage(
					cause,
					t("crew.roomSettings.failed", "Couldn't save the room's changes."),
				),
			);
		}
	};

	return (
		<form
			className="flex flex-col gap-4"
			onSubmit={(event) => {
				event.preventDefault();
				void save();
			}}
		>
			<PicturePicker
				conversation={conversation}
				title={title.trim() || defaultTitle || ""}
				preview={preview}
				disabled={preparing || saving}
				onPick={pick}
				onRemove={() => setStaged(null)}
			/>
			<div className="flex flex-col gap-1.5">
				<Label htmlFor={nameId}>{t("crew.roomSettings.name", "Name")}</Label>
				<Input
					id={nameId}
					value={title}
					maxLength={120}
					onChange={(event) => setTitle(event.target.value)}
					placeholder={
						defaultTitle ||
						t("crew.roomSettings.namePlaceholder", "Name this room")
					}
				/>
			</div>
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			<DialogFooter>
				<Button type="button" variant="outline" onClick={onDone}>
					{t("crew.cancel", "Cancel")}
				</Button>
				<Button
					type="submit"
					disabled={(!titleChanged && !imageChanged) || preparing || saving}
				>
					{t("crew.roomSettings.save", "Save")}
				</Button>
			</DialogFooter>
		</form>
	);
}

/** Renames a group and sets or removes its picture; every member sees both. */
export function RoomSettingsDialog({
	open,
	onOpenChange,
	conversation,
	defaultTitle,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	conversation: IConversation;
	/** What the room is called while it has no name, shown as the placeholder. */
	defaultTitle?: string;
}>) {
	const { t } = useTranslation("bots");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle>{t("crew.roomSettings.title", "Edit room")}</DialogTitle>
					<DialogDescription>
						{t(
							"crew.roomSettings.description",
							"Everyone in the room sees its name and picture.",
						)}
					</DialogDescription>
				</DialogHeader>
				{open && (
					<RoomSettingsForm
						key={conversation.id}
						conversation={conversation}
						defaultTitle={defaultTitle}
						onDone={() => onOpenChange(false)}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}
