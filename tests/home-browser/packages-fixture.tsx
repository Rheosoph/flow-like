import { useEffect, useRef, useState } from "react";
import { HomeEditor } from "../../packages/ui/components/home/home-editor";
import { HomeWidgetContent } from "../../packages/ui/components/home/home-widget-content";
import { HomeWidgetSettings } from "../../packages/ui/components/home/home-widget-settings";
import type {
	IHomeLayout,
	IHomeWidget,
} from "../../packages/ui/components/home/types";
import { PackageCard } from "../../packages/ui/components/store/package-card";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import { defaultFixturePackages } from "./default-fixture-data";

const initialWidget: IHomeWidget = {
	id: "packages-fixture",
	type: "packages",
	title: "Explore packages",
	size: { columns: 12, rows: 4 },
	appearance: { variant: "borderless", accent: "orange" },
	config: { rendering: "standard", limit: 3 },
};
const initialLayout: IHomeLayout = { version: 1, widgets: [initialWidget] };
const metadataCase = {
	...defaultFixturePackages[0],
	id: "qa/paid package?mode=a&team=#1",
	name: "Private connector",
	description:
		"A local fixture with paid access and explicitly empty permissions.",
	metadata: undefined,
	price: 1299,
	latestVersion: "3.2.1",
	ratingCount: 0,
	avgRating: null,
	capabilities: [],
	verified: false,
	visibility: "private",
};

export default function PackagesFixture() {
	const original = useRef(useBackend());
	const [ready, setReady] = useState(false);
	const [widget, setWidget] = useState(initialWidget);
	const editor = new URLSearchParams(window.location.search).has("editor");
	const [layout, setLayout] = useState<IHomeLayout>(() => {
		const saved = sessionStorage.getItem("package-editor-qa-layout");
		return saved ? JSON.parse(saved) : initialLayout;
	});
	useEffect(() => {
		const backend = original.current;
		useBackendStore.getState().setBackend({
			...backend,
			registryState: {
				...backend.registryState,
				searchPackages: async (filters) => {
					if (filters.query === "fail")
						throw new Error("Fixture registry unavailable");
					const packages =
						filters.query === "empty"
							? []
							: defaultFixturePackages.slice(0, filters.limit ?? 3);
					return {
						packages,
						totalCount: packages.length,
						offset: 0,
						limit: filters.limit ?? 3,
					};
				},
			},
		});
		setReady(true);
		return () => useBackendStore.getState().setBackend(backend);
	}, []);
	if (!ready) return <p>Preparing local package fixture…</p>;
	return (
		<main className="min-h-screen space-y-6 bg-background p-4 text-foreground">
			<h1 className="text-xl font-bold">
				Native Explore packages · local fixture
			</h1>
			<p className="text-xs text-muted-foreground">
				Actual shared Explore cards. All registry data and artwork are local
				fixtures.
			</p>
			{editor ? (
				<HomeEditor
					layout={layout}
					defaultLayout={initialLayout}
					onReset={async () => {
						setLayout(initialLayout);
						sessionStorage.removeItem("package-editor-qa-layout");
					}}
					onSave={async (next) => {
						setLayout(next);
						sessionStorage.setItem(
							"package-editor-qa-layout",
							JSON.stringify(next),
						);
					}}
				/>
			) : (
				<>
					<div
						className="max-w-md rounded-xl border p-4"
						data-testid="package-settings"
					>
						<HomeWidgetSettings
							widget={widget}
							onChange={(config) =>
								setWidget((current) => ({ ...current, config }))
							}
						/>
					</div>
					<section data-testid="home-packages">
						<HomeWidgetContent widget={widget} />
					</section>
					<section
						data-testid="package-metadata-cases"
						className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,260px),1fr))] items-start gap-4"
					>
						{(["standard", "compact", "featured"] as const).map((variant) => (
							<PackageCard key={variant} pkg={metadataCase} variant={variant} />
						))}
					</section>
				</>
			)}
			<section
				className="max-w-[350px] space-y-3"
				data-testid="explore-reference"
			>
				<h2 className="font-semibold">Canonical Explore reference</h2>
				<PackageCard pkg={defaultFixturePackages[0]} />
			</section>
		</main>
	);
}
