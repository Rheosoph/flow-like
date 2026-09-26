"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2 } from "lucide-react";
import { useState } from "react";
import {
	EXPLORE_LIMITS,
	type ExploreItemRef,
	type ExploreLayoutDoc,
	type ExplorePlacementInput,
} from "../../../store/explore/explore-types";
import { Button } from "../../../ui/button";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../../ui/dialog";
import {
	type AddChoice,
	type AddTarget,
	choiceLabel,
	errorMessage,
	issueMessage,
	slotLabel,
	validatePlacement,
} from "./explore-admin-model";
import { CountedField } from "./inspector-fields";
import { type ItemNameOf, ItemsEditor } from "./inspector-items";

export interface PendingPlacement {
	target: AddTarget;
	choice: AddChoice;
	input: ExplorePlacementInput;
}

/** Features and sponsored placements need their item (and an advertiser) before the server accepts them. */
export function NewPlacementDialog({
	pending,
	layout,
	refs,
	nameOf,
	onPicked,
	onCancel,
	onCreate,
}: {
	pending: PendingPlacement;
	layout: ExploreLayoutDoc;
	refs: ReadonlyMap<string, ExploreItemRef>;
	nameOf: ItemNameOf;
	onPicked: (key: string, name: string) => void;
	onCancel: () => void;
	onCreate: (target: AddTarget, input: ExplorePlacementInput) => Promise<void>;
}) {
	const { t } = useTranslation("admin");
	const [input, setInput] = useState(pending.input);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const issues = validatePlacement(input);
	const issueFor = (field: string) => {
		const issue = issues.find((entry) => entry.field === field);
		return issue ? issueMessage(issue, t) : undefined;
	};
	const content = input.content;
	const create = async () => {
		setBusy(true);
		setError(null);
		try {
			await onCreate(pending.target, input);
		} catch (reason) {
			setError(
				errorMessage(
					reason,
					t("exploreCreateFailed", "The placement could not be added."),
				),
			);
		} finally {
			setBusy(false);
		}
	};
	return (
		<Dialog open onOpenChange={(open) => !open && !busy && onCancel()}>
			<DialogContent className="flex max-h-[85vh] flex-col sm:max-w-md">
				<DialogHeader>
					<DialogTitle>
						{t("exploreNewPlacementTitle", "Add {{kind}}", {
							kind: choiceLabel(pending.choice, t),
						})}
					</DialogTitle>
					<DialogDescription>
						{t("exploreNewPlacementHint", "It goes to {{slot}} as a draft.", {
							slot: slotLabel(layout, pending.target.slot, t),
						})}
					</DialogDescription>
				</DialogHeader>
				<DialogBody className="flex flex-col gap-3.5">
					<div className="grid grid-cols-2 gap-x-2 gap-y-2.5">
						<CountedField
							label={t("explorePlacementName", "Placement name")}
							value={input.name}
							max={EXPLORE_LIMITS.name}
							error={issueFor("name")}
							onChange={(name) => setInput((current) => ({ ...current, name }))}
						/>
						{content.kind === "sponsored" && (
							<CountedField
								label={t("exploreAdvertiser", "Advertiser")}
								value={content.advertiser}
								max={EXPLORE_LIMITS.advertiser}
								placeholder={t("exploreAdvertiserPlaceholder", "Company name")}
								error={issueFor("advertiser")}
								onChange={(advertiser) =>
									setInput((current) => ({
										...current,
										content: { kind: "sponsored", advertiser },
									}))
								}
							/>
						)}
					</div>
					<ItemsEditor
						title={
							content.kind === "sponsored"
								? t("explorePromotedItem", "Promoted item")
								: t("exploreFeaturedItem", "Featured item")
						}
						addLabel={
							input.items.length
								? t("exploreReplace", "Replace")
								: t("explorePick", "Pick")
						}
						items={input.items}
						kinds={["app", "package"]}
						max={1}
						layout={layout}
						placementId="new"
						refs={refs}
						nameOf={nameOf}
						error={input.items.length ? issueFor("items") : undefined}
						onChange={(items) => setInput((current) => ({ ...current, items }))}
						onPicked={onPicked}
					/>
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
				</DialogBody>
				<DialogFooter>
					<Button
						type="button"
						variant="ghost"
						disabled={busy}
						onClick={onCancel}
					>
						{t("exploreCancel", "Cancel")}
					</Button>
					<Button
						type="button"
						disabled={busy || issues.length > 0}
						onClick={() => void create()}
					>
						{busy && <Loader2 className="size-4 animate-spin" />}
						{t("exploreCreate", "Add placement")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
