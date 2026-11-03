"use client";

import { useTranslation } from "@flow-like/locales";
import { useCreateConversation } from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import {
	type PickedPrincipal,
	type PickerSource,
	PrincipalPicker,
} from "./principal-picker";

const NO_SELECTION: ReadonlySet<string> = new Set();

/** Pick one person or bot; an existing direct conversation opens instead of a new one. */
export function NewMessageDialog({
	open,
	onOpenChange,
	source,
	onPicked,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	source: PickerSource;
	/** Built-in bots open their chat; everyone else gets a direct conversation. */
	onPicked: (target: { bot?: IBot; conversationId?: string }) => void;
}>) {
	const { t } = useTranslation("bots");
	const create = useCreateConversation();

	const pick = async (principal: PickedPrincipal) => {
		if (create.isPending) return;
		if (principal.bot?.system) {
			onOpenChange(false);
			onPicked({ bot: principal.bot });
			return;
		}
		const conversation = await create.mutateAsync({
			kind: "direct",
			members: [principal.id],
		});
		onOpenChange(false);
		onPicked({ conversationId: conversation.id });
	};

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) create.reset();
				onOpenChange(next);
			}}
		>
			<DialogContent className="flex max-h-[85dvh] flex-col sm:max-w-md">
				<DialogHeader>
					<DialogTitle>
						{t("crew.newMessageDialog.title", "New message")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"crew.newMessageDialog.description",
							"Write to one person or one of your bots.",
						)}
					</DialogDescription>
				</DialogHeader>
				{open && (
					<PrincipalPicker
						source={source}
						selected={NO_SELECTION}
						multiple={false}
						full={create.isPending}
						onPick={(principal) => {
							pick(principal).catch(() => undefined);
						}}
					/>
				)}
				{create.isError && (
					<p role="alert" className="text-sm text-destructive">
						{apiErrorMessage(
							create.error,
							t(
								"crew.newMessageDialog.failed",
								"Couldn't start the conversation.",
							),
						)}
					</p>
				)}
			</DialogContent>
		</Dialog>
	);
}
