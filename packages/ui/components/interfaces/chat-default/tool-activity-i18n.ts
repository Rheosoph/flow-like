import type { TFunction } from "i18next";

// Translate built-in labels at render time without rewriting saved activity.
const ACTIVITY_KEYS = {
	"Making an HTTP request": "toolActivity.makingHttpRequest",
	"Reading a webpage": "toolActivity.readingWebpage",
	"Checking the web archive": "toolActivity.checkingWebArchive",
	"Searching the web": "toolActivity.searchingWeb",
	"Researching sources": "toolActivity.researchingSources",
	"Checking available apps": "toolActivity.checkingApps",
	"Searching apps": "toolActivity.searchingApps",
	"Inspecting an app interface": "toolActivity.inspectingAppInterface",
	"Asking an app": "toolActivity.askingApp",
	"Running an app action": "toolActivity.runningAppAction",
	"Opening an app chat": "toolActivity.openingAppChat",
	"Opening an app page": "toolActivity.openingAppPage",
	"Creating an app": "toolActivity.creatingApp",
	"Copying an app": "toolActivity.copyingApp",
	"Working with files": "toolActivity.workingFiles",
	"Reading a file": "toolActivity.readingFile",
	"Listing files": "toolActivity.listingFiles",
	"Working with data": "toolActivity.workingData",
	"Working with forecasts": "toolActivity.workingForecasts",
	"Querying data": "toolActivity.queryingData",
	"Working with app data": "toolActivity.workingAppData",
	"Working on a workflow": "toolActivity.workingWorkflow",
	"Building an interface": "toolActivity.buildingInterface",
	"Updating Home": "toolActivity.updatingHome",
	"Inspecting the project": "toolActivity.inspectingProject",
	"Editing a workflow": "toolActivity.editingWorkflow",
	"Writing a workflow": "toolActivity.writingWorkflow",
	"Updating a workflow": "toolActivity.updatingWorkflow",
	"Checking a workflow": "toolActivity.checkingWorkflow",
	"Testing a workflow": "toolActivity.testingWorkflow",
	"Saving a workflow": "toolActivity.savingWorkflow",
	"Running an event": "toolActivity.runningEvent",
	"Searching the node catalog": "toolActivity.searchingNodeCatalog",
	"Looking up node definitions": "toolActivity.lookingUpNodes",
	"Finding compatible nodes": "toolActivity.findingCompatibleNodes",
	"Requesting your input": "toolActivity.requestingInput",
	"Asking for a secret": "toolActivity.askingSecret",
	"Checking stored secrets": "toolActivity.checkingSecrets",
	"Inviting someone to the chat": "toolActivity.invitingPerson",
	"Running a command": "toolActivity.runningCommand",
	"Asking another bot": "toolActivity.askingBot",
	"Checking forecasting models": "toolActivity.checkingForecastModels",
	"Running a forecast": "toolActivity.runningForecast",
	"Listing saved predictions": "toolActivity.listingPredictions",
	"Opening a saved prediction": "toolActivity.openingPrediction",
	"Saving a prediction": "toolActivity.savingPrediction",
	"Fetching a URL": "toolActivity.fetchingUrl",
	"Sending an HTTP request": "toolActivity.sendingHttpRequest",
	Working: "toolActivity.working",
	Step: "toolActivity.step",
} as const;

export function translateToolActivityLabel(
	label: string,
	t: TFunction<"chat">,
): string {
	if (!Object.hasOwn(ACTIVITY_KEYS, label)) return label;
	const key = ACTIVITY_KEYS[label as keyof typeof ACTIVITY_KEYS];
	return t(key, { defaultValue: label });
}
