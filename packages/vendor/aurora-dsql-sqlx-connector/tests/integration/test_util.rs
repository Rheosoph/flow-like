// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub fn build_conn_str() -> String {
    let endpoint = std::env::var("CLUSTER_ENDPOINT").expect("CLUSTER_ENDPOINT must be set");
    let user = std::env::var("CLUSTER_USER").unwrap_or_else(|_| "admin".to_string());
    format!("postgres://{}@{}/postgres", user, endpoint)
}
