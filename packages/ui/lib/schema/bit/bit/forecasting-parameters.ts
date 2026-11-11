import type { IModelProvider } from "./llm-parameters";

export interface IForecastingModelParameters {
	model_type: IForecastingModelType;
	provider?: IModelProvider;
	assets?: IForecastingAssetRef[];
}

export interface IForecastingAssetRef {
	bit: string;
	relative_path: string;
}

export enum IForecastingModelType {
	Timestral = "timestral",
	Chronos2 = "chronos2",
	Toto2 = "toto2",
	Timesfm25 = "timesfm2_5",
	TinyTimeMixer = "tiny_time_mixer",
}
