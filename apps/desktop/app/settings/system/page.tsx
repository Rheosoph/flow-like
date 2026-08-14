"use client";
import { Trans, useTranslation } from "@flow-like/locales";
import type { UseQueryResult } from "@flow-like/flow-like-ui";
import { humanFileSize } from "@flow-like/flow-like-ui/lib/utils";
import type { ISystemInfo } from "@flow-like/flow-like-ui/types";
import { useTauriInvoke } from "../../../components/useInvoke";

export default function SettingsPage() {
	const { t } = useTranslation("common");
	const systemInfo: UseQueryResult<ISystemInfo> = useTauriInvoke(
		"get_system_info",
		{},
	);

	return (
		<main className="justify-start flex flex-col items-center w-full pr-4 flex-1 min-h-0">
			<div className="flex flex-row items-center justify-between w-full max-w-screen-2xl">
				<h1 className="scroll-m-20 text-4xl font-extrabold tracking-tight lg:text-5xl">
					{t('systemInfo', 'System Info')}
				</h1>
			</div>
			<br />
			<p className="w-full"><Trans i18nKey="bcoresb"><b>Cores</b>:</Trans>{systemInfo.data?.cores}
			</p>
			<p className="w-full"><Trans i18nKey="bvramb"><b>VRAM</b>:</Trans>{humanFileSize(systemInfo.data?.vram ?? 0)}
			</p>
			<p className="w-full"><Trans i18nKey="bramb"><b>RAM</b>:</Trans>{humanFileSize(systemInfo.data?.ram ?? 0)}
			</p>
		</main>
	);
}
