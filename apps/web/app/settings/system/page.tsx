"use client";
import type { UseQueryResult } from "@flow-like/flow-like-ui";
import { humanFileSize } from "@flow-like/flow-like-ui/lib/utils";
import type { ISystemInfo } from "@flow-like/flow-like-ui/types";
import { useTauriInvoke } from "../../../components/useInvoke";

export default function SettingsPage() {
	const systemInfo: UseQueryResult<ISystemInfo> = useTauriInvoke(
		"get_system_info",
		{},
	);

	return (
		<main className="justify-start flex flex-col items-center w-full pr-4 flex-1 min-h-0">
			<div className="flex flex-row items-center justify-between w-full max-w-screen-2xl">
				<h1 className="scroll-m-20 text-4xl font-extrabold tracking-tight lg:text-5xl">
					System Info
				</h1>
			</div>
			<br />
			<p className="w-full">
				<b>Cores</b>: {systemInfo.data?.cores}
			</p>
			<p className="w-full">
				<b>VRAM</b>: {humanFileSize(systemInfo.data?.vram ?? 0)}
			</p>
			<p className="w-full">
				<b>RAM</b>: {humanFileSize(systemInfo.data?.ram ?? 0)}
			</p>
		</main>
	);
}
