"use client";

import {
	type LibraryMachine,
	LibraryPackages,
	PackagesHubPage,
	useBackend,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { type ReactNode, useCallback, useMemo } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { MinePackages } from "../../../components/packages/mine-packages";
import { usePackageStatusMap } from "../../../hooks/use-package-status";
import { fetcher } from "../../../lib/api";

/** Resolves quietly when the dialog is cancelled; rejects when the load fails, which Library reports. */
function useLoadLocalWasm() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	return useCallback(async () => {
		const selected = await open({
			multiple: false,
			filters: [{ name: t("wasmFiles", "WASM Files"), extensions: ["wasm"] }],
		});
		if (typeof selected !== "string") return;
		await backend.registryState.init();
		await invoke("registry_load_local", { path: selected });
		const baseName =
			selected
				.split(/[/\\]/)
				.pop()
				?.replace(/\.wasm$/i, "") ?? "package";
		toast.success(t("loadedBasename", "Loaded {{baseName}}", { baseName }), {
			description: t(
				"packageLoadedForDevelopmentTesting",
				"Package loaded for development testing",
			),
		});
	}, [backend, t]);
}

function PageContent() {
	const auth = useAuth();
	const statusMap = usePackageStatusMap();
	const getPackageStatus = useCallback(
		(packageId: string) => statusMap.get(packageId),
		[statusMap],
	);
	const onLoadLocal = useLoadLocalWasm();
	const machine = useMemo<LibraryMachine>(
		() => ({ getPackageStatus, onLoadLocal }),
		[getPackageStatus, onLoadLocal],
	);
	const mine = useCallback(
		(navigation: ReactNode) => <MinePackages navigation={navigation} />,
		[],
	);
	const library = useCallback(
		(navigation: ReactNode) => (
			<LibraryPackages auth={auth} navigation={navigation} machine={machine} />
		),
		[auth, machine],
	);

	return (
		<PackagesHubPage
			fetcher={fetcher}
			auth={auth}
			getPackageStatus={getPackageStatus}
			mine={mine}
			library={library}
		/>
	);
}

export default function Page() {
	return <PageContent />;
}
