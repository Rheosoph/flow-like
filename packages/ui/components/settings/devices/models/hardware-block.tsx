"use client";

import { useTranslation } from "@flow-like/locales";
import { Cpu, Download, RefreshCw, Trash2 } from "lucide-react";
import type {
	ModelsOverview,
	RuntimeInfo,
	SystemFacts,
} from "../../../../lib/device-management/models";
import { gateView } from "../device/use-device-page";
import { bytesText } from "../observe/observe-data";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import {
	FreshnessStamp,
	type FreshnessStampProps,
} from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { Meter, type MeterTone } from "../primitives/meter";
import { StatusChip } from "../primitives/status-chip";
import { useInlineResults } from "../workspace";
import { backendLabel, runtimeName } from "./models-copy";
import { useRuntimeUpdateCheck } from "./use-models";
import { modelsResultKey, useModelsAction } from "./use-models-action";

const SHOWN_FEATURES = 6;

const ARCH: Record<string, string> = {
	x86_64: "x86-64",
	aarch64: "ARM64",
};

const toneOf = (percent: number): MeterTone =>
	percent >= 90 ? "critical" : percent >= 75 ? "warning" : "neutral";

/** Used against total, with the free amount in words (never colour alone). */
function CapacityMeter({
	used,
	total,
	caption,
}: Readonly<{ used: number; total: number; caption: string }>) {
	const percent = total > 0 ? (used / total) * 100 : 0;
	return (
		<Meter
			className="max-w-[320px]"
			segments={[{ value: percent, tone: toneOf(percent) }]}
			label={caption}
			caption={caption}
		/>
	);
}

function freeOf(t: DevicesT, free: number, total: number) {
	return t("devices:models.hardware.free", "{{free}} free of {{total}}", {
		free: bytesText(free),
		total: bytesText(total),
	});
}

function Processor({ cpu }: Readonly<{ cpu: SystemFacts["cpu"] }>) {
	const { t } = useTranslation("devices");
	const features = cpu.features
		.slice(0, SHOWN_FEATURES)
		.map((feature) => feature.toUpperCase())
		.join(", ");
	return (
		<span className="flex flex-col">
			<span>{cpu.brand}</span>
			<span className="text-xs text-muted-foreground">
				{t("devices:models.hardware.cpuCores", {
					count: cpu.physical_cores,
					arch: ARCH[cpu.arch] ?? cpu.arch,
					defaultValue_one: "{{count, number}} core · {{arch}}",
					defaultValue_other: "{{count, number}} cores · {{arch}}",
				})}
				{features ? ` · ${features}` : null}
			</span>
		</span>
	);
}

function Gpus({ gpus }: Readonly<{ gpus: SystemFacts["gpus"] }>) {
	const { t } = useTranslation("devices");
	if (!gpus.length)
		return (
			<span className="text-muted-foreground">
				{t(
					"devices:models.hardware.noGpu",
					"No GPU found. Models run on the processor.",
				)}
			</span>
		);
	return (
		<ul className="flex flex-col gap-2">
			{gpus.map((gpu, index) => (
				<li
					key={`${index}:${gpu.name}:${gpu.backend}`}
					className="flex flex-col gap-1"
				>
					<span>
						{gpu.name}{" "}
						<span className="text-muted-foreground">
							· {backendLabel(t, gpu.backend)}
							{gpu.backend === "metal"
								? ` · ${t("devices:models.hardware.unified", "memory shared with the processor")}`
								: null}
						</span>
					</span>
					{gpu.memory_total === null || gpu.memory_free === null ? (
						<span className="text-xs text-muted-foreground">
							{t(
								"devices:models.hardware.gpuMemoryUnknown",
								"Memory not reported yet: the device reads it once a runtime for it is installed.",
							)}
						</span>
					) : (
						<CapacityMeter
							used={gpu.memory_total - gpu.memory_free}
							total={gpu.memory_total}
							caption={freeOf(t, gpu.memory_free, gpu.memory_total)}
						/>
					)}
				</li>
			))}
		</ul>
	);
}

function RuntimeRow({
	deviceId,
	runtime,
	replacesInstalled,
}: Readonly<{
	deviceId: string;
	runtime: RuntimeInfo;
	replacesInstalled: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useModelsAction(deviceId);
	const results = useInlineResults(modelsResultKey.runtime(deviceId, runtime));
	const command = runtime.installed
		? actions.removeRuntime(runtime)
		: actions.installRuntime({
				...runtime,
				installed: replacesInstalled,
			});
	const view = gateView(t, time, command.gate);
	return (
		<li
			data-runtime={`${runtime.runtime}-${runtime.backend}`}
			data-runtime-build={runtime.build}
			className="flex flex-col gap-1.5 border-t border-hairline py-2 first:border-t-0 first:pt-0"
		>
			<span className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
				<span className="font-medium">{runtimeName(t, runtime)}</span>
				<span className="font-mono text-xs text-muted-foreground">
					{runtime.build}
				</span>
				{runtime.installed ? (
					<StatusChip tone="good">
						{t("devices:models.hardware.installed", "Installed · {{size}}", {
							size: bytesText(runtime.size),
						})}
					</StatusChip>
				) : (
					<StatusChip tone="outline">
						{t("devices:models.hardware.available", "Available · {{size}}", {
							size: bytesText(runtime.size),
						})}
					</StatusChip>
				)}
				<span className="flex-1" />
				<GatedAction gate={view?.gate}>
					<DvButton
						size="xs"
						variant={runtime.installed ? "danger-ghost" : "default"}
						icon={runtime.installed ? Trash2 : Download}
						busy={command.pending}
						onClick={() => void command.run()}
					>
						{runtime.installed
							? t("devices:models.actions.removeRuntimeButton", "Remove…")
							: replacesInstalled
								? t("devices:models.actions.updateRuntimeButton", "Update…")
								: t("devices:models.actions.installRuntimeButton", "Install…")}
					</DvButton>
				</GatedAction>
			</span>
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</li>
	);
}

/** CPU, memory, GPUs with their memory, the model disk and the runtime packs. */
export function HardwareBlock({
	deviceId,
	overview,
	stamp,
}: Readonly<{
	deviceId: string;
	overview: ModelsOverview;
	stamp: FreshnessStampProps;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const check = useRuntimeUpdateCheck(deviceId, overview.runtime_manifest_url);
	const checkGate = check.supported
		? check.configured
			? gateView(t, time, check.gate)
			: {
					gate: {
						kind: "unsupported" as const,
						reason: t(
							"devices:models.hardware.checkUnconfigured",
							"This device has no trusted runtime release source configured.",
						),
					},
				}
		: {
				gate: {
					kind: "unsupported" as const,
					reason: t(
						"devices:models.hardware.checkUnsupported",
						"Update the device agent to check for runtime updates.",
					),
				},
			};
	const { system, summary, runtimes } = overview;
	const volume = system.model_volume;
	return (
		<Block
			id="models-hardware"
			icon={Cpu}
			title={t("devices:models.hardware.title", "Hardware and runtimes")}
			stamp={<FreshnessStamp {...stamp} />}
			tools={
				<GatedAction gate={checkGate?.gate}>
					<DvButton
						size="sm"
						variant="ghost"
						icon={RefreshCw}
						busy={check.busy}
						onClick={() => void check.run()}
					>
						{t(
							"devices:models.hardware.checkUpdates",
							"Check for runtime updates",
						)}
					</DvButton>
				</GatedAction>
			}
		>
			{check.result && (
				<InlineResult tone={check.result.ok ? "good" : "critical"}>
					{check.result.ok
						? t(
								"devices:models.hardware.checkedUpdates",
								"Runtime releases checked. Available updates appear below.",
							)
						: check.result.error}
				</InlineResult>
			)}
			<KeyValueList>
				<KvRow label={t("devices:models.hardware.cpu", "Processor")}>
					<Processor cpu={system.cpu} />
				</KvRow>
				<KvRow label={t("devices:models.hardware.memory", "Memory")}>
					<CapacityMeter
						used={system.ram.total - system.ram.free}
						total={system.ram.total}
						caption={freeOf(t, system.ram.free, system.ram.total)}
					/>
				</KvRow>
				<KvRow label={t("devices:models.hardware.gpus", "GPUs")}>
					<Gpus gpus={system.gpus} />
				</KvRow>
				<KvRow label={t("devices:models.hardware.storage", "Model disk")}>
					<CapacityMeter
						used={summary.store_bytes}
						total={summary.store_budget_bytes}
						caption={t(
							"devices:models.hardware.storageUsed",
							"{{used}} of {{budget}} used by models · {{free}} free on the disk",
							{
								used: bytesText(summary.store_bytes),
								budget: bytesText(summary.store_budget_bytes),
								free: bytesText(volume.free),
							},
						)}
					/>
				</KvRow>
				<KvRow label={t("devices:models.hardware.runtimes", "Runtimes")}>
					{runtimes.length ? (
						<ul className="flex flex-col">
							{runtimes.map((runtime) => (
								<RuntimeRow
									key={`${runtime.runtime}-${runtime.backend}-${runtime.build}-${runtime.installed}`}
									deviceId={deviceId}
									runtime={runtime}
									replacesInstalled={runtimes.some(
										(installed) =>
											installed.installed &&
											installed.runtime === runtime.runtime &&
											installed.backend === runtime.backend,
									)}
								/>
							))}
						</ul>
					) : (
						<span className="text-muted-foreground">
							{t(
								"devices:models.hardware.noRuntimes",
								"No runtime is offered for this device yet.",
							)}
						</span>
					)}
				</KvRow>
			</KeyValueList>
		</Block>
	);
}
