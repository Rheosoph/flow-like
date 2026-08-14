"use client";

import { i18n as i18next } from "@flow-like/locales";
import * as React from "react";

import { PlaceholderPlugin, UploadErrorCode } from "@platejs/media/react";
import { usePluginOption } from "platejs/react";
import { toast } from "sonner";

export function MediaUploadToast() {
	useUploadErrorToast();

	return null;
}

const useUploadErrorToast = () => {
	const uploadError = usePluginOption(PlaceholderPlugin, "error");

	React.useEffect(() => {
		if (!uploadError) return;

		const { code, data } = uploadError;

		switch (code) {
			case UploadErrorCode.INVALID_FILE_SIZE: {
				toast.error(
					i18next.t('theSizeOfFilesValIsInvalid', 'The size of files {{val}} is invalid', { val: data.files
						.map((f) => f.name)
						.join(", ") }),
				);

				break;
			}
			case UploadErrorCode.INVALID_FILE_TYPE: {
				toast.error(
					i18next.t('theTypeOfFilesValIsInvalid', 'The type of files {{val}} is invalid', { val: data.files
						.map((f) => f.name)
						.join(", ") }),
				);

				break;
			}
			case UploadErrorCode.TOO_LARGE: {
				toast.error(
					i18next.t('theSizeOfFilesValIsTooLargeThanMaxfilesize', 'The size of files {{val}} is too large than {{maxFileSize}}', { val: data.files
						.map((f) => f.name)
						.join(", "), maxFileSize: data.maxFileSize }),
				);

				break;
			}
			case UploadErrorCode.TOO_LESS_FILES: {
				toast.error(
					i18next.t('theMiniUmNumberOfFilesIsMinfilecountForFiletype', 'The mini um number of files is {{minFileCount}} for {{fileType}}', { minFileCount: data.minFileCount, fileType: data.fileType }),
				);

				break;
			}
			case UploadErrorCode.TOO_MANY_FILES: {
				toast.error(
					i18next.t('theMaximumNumberOfFilesIsMaxfilecountVal', 'The maximum number of files is {{maxFileCount}} {{val}}', { maxFileCount: data.maxFileCount, val: data.fileType ? `for ${data.fileType}` : "" }),
				);

				break;
			}
		}
	}, [uploadError]);
};
