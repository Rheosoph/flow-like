import type { IEventState } from "../../packages/ui/state/backend-state/event-state";

let execute: IEventState["executeEvent"] = async () => {
	throw new Error("Benchmark backend is not ready");
};

const backend = {
	boardState: {},
	eventState: {
		executeEvent: (...args: Parameters<IEventState["executeEvent"]>) =>
			execute(...args),
	},
	widgetState: {
		getWidget: async () => {
			throw new Error(
				"Benchmark widget must be carried inline by the executor",
			);
		},
	},
};

export function installExecuteEvent(next: IEventState["executeEvent"]) {
	execute = next;
}
export const useBackend = () => backend;
