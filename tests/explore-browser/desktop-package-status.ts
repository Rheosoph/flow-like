import type { CompileStatus } from "../../packages/ui/components/ui/package-status-badge";

const statuses = new Map<string, CompileStatus>([
	["explore-package-1", "ready"],
	["explore-package-2", "ready"],
]);
export function usePackageStatusMap() {
	return statuses;
}
export function usePackageStatus(id: string) {
	return statuses.get(id) ?? "idle";
}
