const backend = {
	storageState: {
		downloadStorageItems: async () => [],
		downloadStorageItemsUser: async () => [],
	},
};

export const useBackend = () => backend;
export const useBackendReady = () => true;
export const useSignedIn = () => false;
