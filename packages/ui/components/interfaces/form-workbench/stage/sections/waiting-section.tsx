"use client";

import { useTranslation } from "@flow-like/locales";
import type { IInteractionRequest } from "../../../../../lib/schema/interaction";
import { Interaction } from "../../../chat-default/interaction";
import { SectionLabel } from "../section-label";

/**
 * "Waiting for you": the questions the flow asked in the run (single choice, several choices, a form). The
 * chat's own cards answer them; the answer goes back through the session (`respondInteraction`).
 */
export function WaitingSection({
	interactions,
	onRespond,
}: Readonly<{
	interactions: readonly IInteractionRequest[];
	onRespond: (interactionId: string, value: unknown) => void;
}>) {
	const { t } = useTranslation("interfaces");
	return (
		<section className="flex flex-col gap-2.5">
			<SectionLabel>
				{t("workbench.stage.waiting.label", "Waiting for you")}
			</SectionLabel>
			{interactions.map((interaction) => (
				<Interaction
					key={interaction.id}
					interaction={interaction}
					onRespond={onRespond}
				/>
			))}
		</section>
	);
}
