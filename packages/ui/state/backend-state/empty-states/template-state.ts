import type {
	IBoard,
	IMetadata,
	ITemplateState,
	IVersionType,
} from "@flow-like/flow-like-ui";

export class EmptyTemplateState implements ITemplateState {
	getTemplates(
		appId?: string,
		language?: string,
	): Promise<[string, string, IMetadata | undefined][]> {
		throw new Error("Method not implemented.");
	}
	getTemplate(
		appId: string,
		templateId: string,
		version?: [number, number, number],
	): Promise<IBoard> {
		throw new Error("Method not implemented.");
	}
	upsertTemplate(
		appId: string,
		boardId: string,
		templateId?: string,
		boardVersion?: [number, number, number],
		versionType?: IVersionType,
	): Promise<[string, [number, number, number]]> {
		throw new Error("Method not implemented.");
	}
	deleteTemplate(appId: string, templateId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getTemplateMeta(
		appId: string,
		templateId: string,
		language?: string,
	): Promise<IMetadata> {
		throw new Error("Method not implemented.");
	}
	pushTemplateMeta(
		appId: string,
		templateId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
