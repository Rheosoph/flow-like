"use client";

import {
	DiffDemo,
	FaceVisionDemo,
	PlanningDemo,
	SchemaInputsDemo,
	UserProfileDemo,
	VoiceStudioDemo,
} from "./demos/A2UIProductDemos";
import {
	AppConnectionsDemo,
	DataStudioTourDemo,
	GraphAnalyticsDemo,
	ProcessCasesDemo,
	ProcessLineageDemo,
	RemoteOntologyDemo,
	SuiteConsoleDemo,
	SuiteLibraryDemo,
	SuitePublicationDemo,
} from "./demos/PlatformProductDemos";
import {
	BoardSyncDemo,
	HomeExploreDemo,
	RenderPerformanceDemo,
	StorageOverviewDemo,
	StorageRetentionDemo,
} from "./demos/SystemProductDemos";

export type BlogInteractiveDemoPreset =
	| "planning"
	| "diff"
	| "schema-inputs"
	| "user-profile"
	| "face-vision"
	| "voice-studio"
	| "app-connections"
	| "process-lineage"
	| "process-cases"
	| "suite-console"
	| "suite-library"
	| "suite-publication"
	| "data-studio-tour"
	| "graph-analytics"
	| "remote-ontology"
	| "board-sync"
	| "render-performance"
	| "storage-overview"
	| "storage-retention"
	| "home-explore";

export interface BlogInteractiveDemoProps {
	preset: BlogInteractiveDemoPreset;
}

export function BlogInteractiveDemo({
	preset,
}: Readonly<BlogInteractiveDemoProps>) {
	switch (preset) {
		case "planning":
			return <PlanningDemo />;
		case "diff":
			return <DiffDemo />;
		case "schema-inputs":
			return <SchemaInputsDemo />;
		case "user-profile":
			return <UserProfileDemo />;
		case "face-vision":
			return <FaceVisionDemo />;
		case "voice-studio":
			return <VoiceStudioDemo />;
		case "app-connections":
			return <AppConnectionsDemo />;
		case "process-lineage":
			return <ProcessLineageDemo />;
		case "process-cases":
			return <ProcessCasesDemo />;
		case "suite-console":
			return <SuiteConsoleDemo />;
		case "suite-library":
			return <SuiteLibraryDemo />;
		case "suite-publication":
			return <SuitePublicationDemo />;
		case "data-studio-tour":
			return <DataStudioTourDemo />;
		case "graph-analytics":
			return <GraphAnalyticsDemo />;
		case "remote-ontology":
			return <RemoteOntologyDemo />;
		case "board-sync":
			return <BoardSyncDemo />;
		case "render-performance":
			return <RenderPerformanceDemo />;
		case "storage-overview":
			return <StorageOverviewDemo />;
		case "storage-retention":
			return <StorageRetentionDemo />;
		case "home-explore":
			return <HomeExploreDemo />;
	}
}
