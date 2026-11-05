import { replyToChannel } from "../../../lib/channel";
import { getExecutionClientForChannel } from "../../../lib/execution/runtime-client";
import type { IInteractionRequest } from "../../../lib/schema/interaction";

/**
 * Submit a user's response to a chat interaction (single/multiple choice, form) on the channel
 * the request arrived with. Throws on failure. Callers own the optimistic state update and error
 * surface.
 */
export async function submitInteractionResponse(
	interaction: IInteractionRequest,
	value: unknown,
): Promise<{ value: unknown }> {
	if (!interaction.channel) {
		throw new Error(
			`Interaction '${interaction.id}' carries no channel to answer on.`,
		);
	}
	await replyToChannel(interaction.channel, value);
	const owner = getExecutionClientForChannel(interaction.channel);
	const accepted = owner
		?.getTasks()
		.flatMap((task) => task.pendingInteractions)
		.find(
			(request) =>
				request.channel?.channel_id === interaction.channel?.channel_id &&
				request.channel?.request_id === interaction.channel?.request_id,
		);
	return {
		value: accepted?.status === "submitted" ? accepted.response_value : value,
	};
}
