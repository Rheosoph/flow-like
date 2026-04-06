//! Test stubs for WIT-generated bindings.
//!
//! During `cargo test` the real WIT bindings are not available (no WASM host).
//! This module mirrors the module hierarchy that `wit_bindgen::generate!()` would
//! create so that `host.rs` can `use crate::wit_stub::flow_like::node::*` and
//! compile without errors.

pub mod flow_like {
    pub mod node {
        pub mod logging {
            pub fn log(_level: u8, _message: &str) {}
        }

        pub mod pins {
            pub fn get_input(_name: &str) -> Option<String> {
                None
            }
            pub fn set_output(_name: &str, _value: &str) {}
            pub fn activate_exec(_name: &str) {}
        }

        pub mod variables {
            pub fn get_var(_name: &str) -> Option<String> {
                None
            }
            pub fn set_var(_name: &str, _value: &str) {}
            pub fn delete_var(_name: &str) {}
            pub fn has_var(_name: &str) -> bool {
                false
            }
        }

        pub mod cache {
            pub fn cache_get(_key: &str) -> Option<String> {
                None
            }
            pub fn cache_set(_key: &str, _value: &str) {}
            pub fn cache_delete(_key: &str) {}
            pub fn cache_has(_key: &str) -> bool {
                false
            }
        }

        pub mod streaming {
            pub fn emit(_event_type: &str, _data: &str) {}
            pub fn text(_content: &str) {}
        }

        pub mod metadata {
            pub fn get_node_id() -> String {
                "stub-node".into()
            }
            pub fn get_run_id() -> String {
                "stub-run".into()
            }
            pub fn get_app_id() -> String {
                "stub-app".into()
            }
            pub fn get_board_id() -> String {
                "stub-board".into()
            }
            pub fn get_user_id() -> String {
                "stub-user".into()
            }
            pub fn time_now() -> u64 {
                0
            }
            pub fn random() -> f64 {
                0.0
            }
            pub fn is_streaming() -> bool {
                false
            }
            pub fn get_log_level() -> u8 {
                0
            }
        }

        pub mod storage {
            pub fn storage_dir(_node_scoped: bool) -> Option<String> {
                None
            }
            pub fn upload_dir() -> Option<String> {
                None
            }
            pub fn cache_dir(_node_scoped: bool, _user_scoped: bool) -> Option<String> {
                None
            }
            pub fn user_dir(_node_scoped: bool) -> Option<String> {
                None
            }
            pub fn read_file(_flow_path: &str) -> Option<Vec<u8>> {
                None
            }
            pub fn write_file(_flow_path: &str, _data: &[u8]) -> bool {
                false
            }
            pub fn list_files(_flow_path: &str) -> Option<String> {
                None
            }
        }

        pub mod models {
            pub fn embed_text(_bit_json: &str, _texts_json: &str) -> Option<String> {
                None
            }
            pub fn embed_text_query(_model_json: &str, _texts_json: &str) -> Option<String> {
                None
            }
            pub fn embed_text_document(_model_json: &str, _texts_json: &str) -> Option<String> {
                None
            }
            pub fn embed_image(_model_json: &str, _image_data: &[u8]) -> Option<String> {
                None
            }
            pub fn llm_prompt(
                _bit_json: &str,
                _messages_json: &str,
                _stream: bool,
            ) -> Option<String> {
                None
            }
            pub fn llm_prompt_stream(
                _bit_json: &str,
                _request_json: &str,
            ) -> Option<String> {
                None
            }
        }

        pub mod auth {
            pub fn get_oauth_token(_provider: &str) -> Option<String> {
                None
            }
            pub fn has_oauth_token(_provider: &str) -> bool {
                false
            }
        }

        pub mod http {
            pub fn request(
                _method: u8,
                _url: &str,
                _headers: &str,
                _body: Option<&[u8]>,
            ) -> Option<String> {
                None
            }
        }

        pub mod schema {
            pub fn get_type_schema(_type_name: &str) -> Option<String> {
                None
            }
            pub fn list_types() -> Option<String> {
                None
            }
        }

        pub mod image {
            pub fn from_bytes(_data: &[u8], _format: &str) -> Option<String> {
                None
            }
            pub fn to_bytes(_image_ref: &str, _format: &str) -> Option<Vec<u8>> {
                None
            }
        }

        pub mod db {
            pub fn query(
                _op: u32,
                _connection_json: &str,
                _payload_json: &str,
            ) -> Option<String> {
                None
            }
        }

        pub mod websocket {
            pub fn connect(_url: &str, _headers_json: &str) -> Option<String> {
                None
            }
            pub fn send(_session_id: &str, _message: &[u8], _is_binary: bool) -> bool {
                false
            }
            pub fn receive(_session_id: &str, _timeout_ms: u32) -> Option<String> {
                None
            }
            pub fn close(_session_id: &str) -> bool {
                false
            }
        }
    }
}
