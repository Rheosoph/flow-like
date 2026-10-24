import { createContext, useCallback, useContext, useState } from "react";
import type { ReactNode } from "react";

export const qa = {
	underlyingClicks: 0,
	saved: [] as string[],
	open: () => {},
};

const initialJson = JSON.stringify({ components: [], widgetRefs: {} }, null, 2);
const toolbarState = {
	copy: () => {},
	cut: () => {},
	paste: () => {},
	deleteComponents: (_ids: string[]) => {},
	clipboard: null,
	selection: { componentIds: [] as string[] },
};
const BuilderContext = createContext({
	...toolbarState,
	devMode: true,
	setDevMode: (_value: boolean) => {},
	getRawJson: () => initialJson,
	setRawJson: (_value: string) => true,
});

export const useBuilder = () => useContext(BuilderContext);

export function BuilderFixture({ children }: { children: ReactNode }) {
	const [devMode, setDevMode] = useState(false);
	const [appliedJson, setAppliedJson] = useState(initialJson);
	const getRawJson = useCallback(() => appliedJson, [appliedJson]);
	const setRawJson = useCallback((value: string) => {
		try {
			JSON.parse(value);
			qa.saved.push(value);
			setAppliedJson(value);
			return true;
		} catch {
			return false;
		}
	}, []);
	qa.open = () => setDevMode(true);
	return (
		<BuilderContext.Provider
			value={{ ...toolbarState, devMode, setDevMode, getRawJson, setRawJson }}
		>
			{children}
		</BuilderContext.Provider>
	);
}
