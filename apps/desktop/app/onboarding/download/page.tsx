"use client";
import { invoke } from "@tauri-apps/api/core";
import {
	type IHub,
	type UseQueryResult,
	useBackend,
	useDownloadManager,
	useInvalidateInvoke,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
} from "@flow-like/flow-like-ui/components/ui/avatar";
import { BitHover } from "@flow-like/flow-like-ui/components/ui/bit-hover";
import {
	type ChartConfig,
	ChartContainer,
	ChartTooltip,
	ChartTooltipContent,
} from "@flow-like/flow-like-ui/components/ui/chart";
import type { IBit } from "@flow-like/flow-like-ui/lib/schema/bit/bit";
import { humanFileSize } from "@flow-like/flow-like-ui/lib/utils";
import type { ISettingsProfile } from "@flow-like/flow-like-ui/types";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { CartesianGrid, Line, LineChart, XAxis } from "recharts";
import { useTauriInvoke } from "../../../components/useInvoke";

const chartConfig = {
	downloaded: {
		label: "Speed (Mbit/s)",
		color: "var(--chart-3)",
	},
	total: {
		label: "Total",
		color: "var(--chart-4)",
	},
} satisfies ChartConfig;

export default function ProfileCreation() {
	const backend = useBackend();
	const { manager } = useDownloadManager();
	const invalidate = useInvalidateInvoke();
	const router = useRouter();
	const queryClient = useQueryClient();
	const intervalRef = useRef<any>(null);
	const searchParams = useSearchParams();
	const [stats, setStats] = useState<
		{
			time: number;
			timeString: string;
			speed: number;
			total: number;
			progress: number;
			max: number;
		}[]
	>([]);
	const [doneCounter, setDoneCounter] = useState(0);
	const defaultProfiles: UseQueryResult<[[ISettingsProfile, IBit[]][], IHub]> =
		useTauriInvoke("get_default_profiles", {});

	const [bits, setBits] = useState<IBit[]>([]);
	const [filter, setFilter] = useState(new Set<string>());
	const [totalSize, setTotalSize] = useState(0);

	async function addProfiles(profiles: ISettingsProfile[]) {
		for await (const profile of profiles) {
			await invoke("upsert_profile", { profile });
		}

		await invalidate(backend.userState.getProfile, []);
		await invalidate(backend.userState.getSettingsProfile, []);
		await queryClient.invalidateQueries({
			queryKey: ["get", "profiles"],
		});
	}

	useEffect(() => {
		if (!defaultProfiles.data) return;
		const selected = new Set(
			searchParams
				.getAll("profiles")
				.map((profile) => profile.replaceAll("profiles=", "")),
		);
		if (selected.size === 0) router.push("/onboarding");

		const filteredProfiles = defaultProfiles.data[0].filter((profile) =>
			selected.has(profile[0].hub_profile.id ?? ""),
		);
		const foundBits = new Map<string, IBit>();
		const filter = new Set<string>();
		filteredProfiles.forEach(([profile, bits]) => {
			bits.forEach((bit) => {
				foundBits.set(bit.id, bit);
				filter.add(bit.hash);
			});
		});

		setBits(Array.from(foundBits.values()));
		setFilter(filter);
		addProfiles(filteredProfiles.map(([profile]) => profile));
	}, [defaultProfiles.data, searchParams, router]);

	useEffect(() => {
		calculateStats();

		intervalRef.current = setInterval(() => {
			calculateStats();
		}, 1000);

		return () => {
			clearInterval(intervalRef.current);
		};
	}, [filter]);

	useEffect(() => {
		async function finalize() {
			router.push("/onboarding/done");
		}
		if (doneCounter >= 5 && bits.length > 0) finalize();
	}, [doneCounter, bits, router]);

	const calculateStats = useCallback(async () => {
		const measurement = await manager.getSpeed(filter);
		console.groupCollapsed("Download Measurement");
		console.dir({
			measurement,
			filter,
			manager,
		});
		console.groupEnd();
		setTotalSize((prev) => Math.max(prev, measurement.max));
		const time = Date.now();
		const timeString = new Date(time).toLocaleTimeString();
		setStats((prev) => {
			const next = [...prev];
			next.push({
				time,
				timeString,
				speed: measurement.bytesPerSecond,
				progress: measurement.progress,
				total: measurement.total,
				max: measurement.max,
			});

			if (next.length > 10) {
				next.shift();
			}

			return next;
		});

		if (measurement.total >= measurement.max) {
			setDoneCounter((prev) => prev + 1);
			console.log("Done Counter", doneCounter);
			return;
		}

		setDoneCounter(0);
	}, [doneCounter, filter, manager]);

	return (
		<div className="min-h-screen w-full flex items-center justify-center">
			<div className="p-3 sm:p-4 max-w-screen-lg w-full z-10 mx-auto">
				<div className="flex flex-col sm:flex-row items-start sm:items-center gap-2">
					<div className="w-full">
						<h1>🚀 Great Selection!</h1>
						<h2>
							Let&apos;s download your models{" "}
							<b className="highlight">
								{(stats[stats.length - 1]?.progress || 0).toFixed(2)}%
							</b>{" "}
							finished 🤩
						</h2>
					</div>
				</div>
				<div className="mt-3 sm:mt-4">
					<ChartContainer
						className="h-[240px] sm:h-[300px] w-full z-20"
						config={chartConfig}
					>
						<LineChart
							accessibilityLayer
							data={stats}
							margin={{
								left: 12,
								right: 12,
							}}
						>
							<CartesianGrid vertical={false} />
							<XAxis
								dataKey="timeString"
								tickLine={false}
								axisLine={false}
								tickMargin={8}
							/>
							<ChartTooltip cursor={false} content={<ChartTooltipContent />} />
							<Line
								dataKey="speed"
								type="monotone"
								allowReorder="yes"
								animateNewValues={false}
								animationEasing="ease-in-out"
								animationDuration={100}
								stroke="var(--color-downloaded)"
								strokeWidth={2}
								dot={false}
							/>
						</LineChart>
					</ChartContainer>
				</div>
				<div className="flex flex-col sm:flex-row justify-between items-start sm:items-center gap-3 sm:gap-4 mt-4 pb-6">
					<div className="flex flex-row items-center justify-start flex-wrap gap-2 max-w-full">
						{bits.map((bit) => (
							<BitDownload key={bit.id} bit={bit} />
						))}
					</div>
					<div className="flex flex-row flex-wrap gap-2 justify-end items-center">
						<div className="border p-2 bg-card text-card-foreground">
							{humanFileSize(totalSize)} Total
						</div>
						<div className="border p-2 bg-card text-card-foreground">
							{humanFileSize(stats[stats.length - 1]?.speed ?? 0)} / s
						</div>
						<button
							onClick={() => {
								router.push("/");
							}}
							className="border p-2 bg-primary text-primary-foreground hover:bg-background hover:text-foreground transition-all"
						>
							Background Download
						</button>
					</div>
				</div>
			</div>
		</div>
	);
}

function BitDownload({ bit }: Readonly<{ bit: IBit }>) {
	const backend = useBackend();
	const { download } = useDownloadManager();
	useEffect(() => {
		if (!backend) return;
		const downloadBit = async () => {
			try {
				await download(bit);
			} catch (error) {
				console.error(error);
			}
		};

		downloadBit();
	}, [bit, backend, download]);

	return (
		<BitHover bit={bit}>
			<Avatar className="border">
				<AvatarImage
					className="p-1 transition-transform duration-200 ease-in-out transform scale-110 group-hover:scale-150 rounded-full"
					src={bit.meta?.en?.icon ?? "/app-logo.webp"}
				/>
				<AvatarFallback>NA</AvatarFallback>
			</Avatar>
		</BitHover>
	);
}
