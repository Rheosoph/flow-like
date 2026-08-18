import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";

/**
 * Capability tags come off `PackageSummary.capabilities`, derived server-side
 * from the manifest permissions. `elevated` marks the ones that let a package
 * reach beyond its sandbox — network, OAuth, model budget, user storage — so
 * listing surfaces can keep those visible when they truncate.
 */
export type CapabilitySeverity = "elevated" | "standard";

export interface ResolvedCapability {
	key: string;
	label: string;
	severity: CapabilitySeverity;
}

const ELEVATED = new Set([
	"net.http",
	"net.ws",
	"net.tcp",
	"net.udp",
	"net.dns",
	"oauth",
	"models",
	"storage.user",
]);

export function capabilitySeverity(key: string): CapabilitySeverity {
	return ELEVATED.has(key) ? "elevated" : "standard";
}

export function usePackageCapabilities(
	keys: readonly string[] | undefined,
): ResolvedCapability[] {
	const { t } = useTranslation("store");

	return useMemo(() => {
		if (!keys?.length) return [];

		const labels: Record<string, string> = {
			"net.http": t("capabilityNetHttp", "Makes HTTP requests"),
			"net.ws": t("capabilityNetWs", "Opens WebSocket connections"),
			"net.tcp": t("capabilityNetTcp", "Opens TCP connections"),
			"net.udp": t("capabilityNetUdp", "Opens UDP connections"),
			"net.dns": t("capabilityNetDns", "Resolves DNS names"),
			oauth: t("capabilityOauth", "Acts on your behalf via OAuth"),
			models: t("capabilityModels", "Calls language models"),
			"storage.user": t("capabilityStorageUser", "Reads and writes your files"),
			"storage.node": t("capabilityStorageNode", "Uses node-scoped storage"),
			"storage.uploads": t("capabilityStorageUploads", "Reads uploaded files"),
			"storage.cache": t("capabilityStorageCache", "Uses the cache directory"),
			variables: t("capabilityVariables", "Reads execution variables"),
			cache: t("capabilityCache", "Uses the execution cache"),
			streaming: t("capabilityStreaming", "Streams output"),
			a2ui: t("capabilityA2ui", "Renders interface elements"),
		};

		return keys.map((key) => ({
			key,
			label: labels[key] ?? key,
			severity: capabilitySeverity(key),
		}));
	}, [keys, t]);
}
