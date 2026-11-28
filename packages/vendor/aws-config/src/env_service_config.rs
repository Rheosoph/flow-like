/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_runtime::env_config::section::EnvConfigSections;
use aws_runtime::env_config::EnvConfigValue;
use aws_types::os_shim_internal::Env;
use aws_types::service_config::{LoadServiceConfig, ServiceConfigKey};

/// The environment variable key used for endpoint URL configuration.
const ENDPOINT_URL_ENV_KEY: &str = "AWS_ENDPOINT_URL";

#[derive(Debug)]
pub(crate) struct EnvServiceConfig {
    pub(crate) env: Env,
    pub(crate) env_config_sections: EnvConfigSections,
    pub(crate) ignore_configured_endpoint_urls: bool,
}

impl LoadServiceConfig for EnvServiceConfig {
    fn load_config(&self, key: ServiceConfigKey<'_>) -> Option<String> {
        if self.ignore_configured_endpoint_urls && key.env() == ENDPOINT_URL_ENV_KEY {
            return None;
        }

        let (value, _source) = EnvConfigValue::new()
            .env(key.env())
            .profile(key.profile())
            .service_id(key.service_id())
            .load(&self.env, Some(&self.env_config_sections))?;

        Some(value.to_string())
    }
}
