import { replyToChannel } from "../../../lib/channel";
import type { IInteractionRequest } from "../../../lib/schema/interaction";

/**
 * Submit a user's response to a chat interaction (single/multiple choice, form) on the channel
 * the request arrived with. Throws on failure — callers own the optimistic state update and error
 * surface.
 */
export async function submitInteractionResponse(
	interaction: IInteractionRequest,
	value: unknown,
): Promise<void> {
	if (!interaction.channel) {
		throw new Error(
			`Interaction '${interaction.id}' carries no channel to answer on.`,
		);
	}
	await replyToChannel(interaction.channel, value);
}
