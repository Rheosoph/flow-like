import type { RefObject } from "react";
import type { ImperativePanelHandle } from "react-resizable-panels";

interface UseFlowPanelsProps {
	varPanelRef: RefObject<ImperativePanelHandle | null>;
	logPanelRef: RefObject<ImperativePanelHandle | null>;
	setVarsOpen: (value: boolean | ((v: boolean) => boolean)) => void;
	setLogsOpen: (value: boolean | ((v: boolean) => boolean)) => void;
}

export function useFlowPanels({
	varPanelRef,
	logPanelRef,
	setVarsOpen,
	setLogsOpen,
}: UseFlowPanelsProps) {
	const toggleVars = () => {
		if (
			typeof window !== "undefined" &&
			window.matchMedia("(max-width: 767px)").matches
		) {
			setVarsOpen((v) => !v);
			return;
		}
		if (!varPanelRef.current) return;
		const isCollapsed = varPanelRef.current.isCollapsed();
		isCollapsed ? varPanelRef.current.expand() : varPanelRef.current.collapse();

		if (!isCollapsed) return;

		const size = varPanelRef.current.getSize();
		if (size < 10) varPanelRef.current.resize(20);
	};

	const toggleLogs = () => {
		if (
			typeof window !== "undefined" &&
			window.matchMedia("(max-width: 767px)").matches
		) {
			setLogsOpen((v) => !v);
			return;
		}
		if (!logPanelRef.current) return;
		const isCollapsed = logPanelRef.current.isCollapsed();
		isCollapsed ? logPanelRef.current.expand() : logPanelRef.current.collapse();

		if (!isCollapsed) return;

		const size = logPanelRef.current.getSize();
		if (size < 10) logPanelRef.current.resize(20);
	};

	return {
		toggleVars,
		toggleLogs,
	};
}
