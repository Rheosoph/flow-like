import { createStore } from "zustand/vanilla";
import type { WorkspaceStore } from "./types";

/** Version counter the React bindings memoise on (CA10); managers bump it on every snapshot change. */
export function createWorkspaceStore(): WorkspaceStore {
	const store = createStore<{ version: number }>(() => ({ version: 0 }));
	return {
		getVersion: () => store.getState().version,
		bump: () => store.setState(({ version }) => ({ version: version + 1 })),
		subscribe: (listener) => store.subscribe(() => listener()),
	};
}
