#pragma once
/* Flow-Like host function imports for WASM (Nim SDK) */

/* flowlike_log */
__attribute__((import_module("flowlike_log"), import_name("trace")))
void fl_log_trace(const char* p, unsigned int l);
__attribute__((import_module("flowlike_log"), import_name("debug")))
void fl_log_debug(const char* p, unsigned int l);
__attribute__((import_module("flowlike_log"), import_name("info")))
void fl_log_info(const char* p, unsigned int l);
__attribute__((import_module("flowlike_log"), import_name("warn")))
void fl_log_warn(const char* p, unsigned int l);
__attribute__((import_module("flowlike_log"), import_name("error")))
void fl_log_error(const char* p, unsigned int l);
__attribute__((import_module("flowlike_log"), import_name("log_json")))
void fl_log_json(int level, const char* msg_p, unsigned int msg_l, const char* data_p, unsigned int data_l);

/* flowlike_pins */
__attribute__((import_module("flowlike_pins"), import_name("get_input")))
long long fl_get_input(const char* name_p, unsigned int name_l);
__attribute__((import_module("flowlike_pins"), import_name("set_output")))
void fl_set_output(const char* name_p, unsigned int name_l, const char* val_p, unsigned int val_l);
__attribute__((import_module("flowlike_pins"), import_name("activate_exec")))
void fl_activate_exec(const char* name_p, unsigned int name_l);

/* flowlike_vars */
__attribute__((import_module("flowlike_vars"), import_name("get")))
long long fl_var_get(const char* name_p, unsigned int name_l);
__attribute__((import_module("flowlike_vars"), import_name("set")))
void fl_var_set(const char* name_p, unsigned int name_l, const char* val_p, unsigned int val_l);
__attribute__((import_module("flowlike_vars"), import_name("delete")))
void fl_var_delete(const char* name_p, unsigned int name_l);
__attribute__((import_module("flowlike_vars"), import_name("has")))
int fl_var_has(const char* name_p, unsigned int name_l);

/* flowlike_cache */
__attribute__((import_module("flowlike_cache"), import_name("get")))
long long fl_cache_get(const char* key_p, unsigned int key_l);
__attribute__((import_module("flowlike_cache"), import_name("set")))
void fl_cache_set(const char* key_p, unsigned int key_l, const char* val_p, unsigned int val_l);
__attribute__((import_module("flowlike_cache"), import_name("delete")))
void fl_cache_delete(const char* key_p, unsigned int key_l);
__attribute__((import_module("flowlike_cache"), import_name("has")))
int fl_cache_has(const char* key_p, unsigned int key_l);

/* flowlike_meta */
__attribute__((import_module("flowlike_meta"), import_name("get_node_id")))
long long fl_get_node_id(void);
__attribute__((import_module("flowlike_meta"), import_name("get_run_id")))
long long fl_get_run_id(void);
__attribute__((import_module("flowlike_meta"), import_name("get_app_id")))
long long fl_get_app_id(void);
__attribute__((import_module("flowlike_meta"), import_name("get_board_id")))
long long fl_get_board_id(void);
__attribute__((import_module("flowlike_meta"), import_name("get_user_id")))
long long fl_get_user_id(void);
__attribute__((import_module("flowlike_meta"), import_name("is_streaming")))
int fl_is_streaming(void);
__attribute__((import_module("flowlike_meta"), import_name("get_log_level")))
int fl_get_log_level(void);
__attribute__((import_module("flowlike_meta"), import_name("time_now")))
long long fl_time_now(void);
__attribute__((import_module("flowlike_meta"), import_name("random")))
long long fl_random(void);

/* flowlike_storage */
__attribute__((import_module("flowlike_storage"), import_name("read_request")))
long long fl_storage_read(const char* path_p, unsigned int path_l);
__attribute__((import_module("flowlike_storage"), import_name("write_request")))
int fl_storage_write(const char* path_p, unsigned int path_l, const char* data_p, unsigned int data_l);
__attribute__((import_module("flowlike_storage"), import_name("storage_dir")))
long long fl_storage_dir(int node_scoped);
__attribute__((import_module("flowlike_storage"), import_name("upload_dir")))
long long fl_upload_dir(void);
__attribute__((import_module("flowlike_storage"), import_name("cache_dir")))
long long fl_cache_dir(int node_scoped, int user_scoped);
__attribute__((import_module("flowlike_storage"), import_name("user_dir")))
long long fl_user_dir(int node_scoped);
__attribute__((import_module("flowlike_storage"), import_name("list_request")))
long long fl_storage_list(const char* path_p, unsigned int path_l);

/* flowlike_models */
__attribute__((import_module("flowlike_models"), import_name("embed_text")))
long long fl_embed_text(const char* bit_p, unsigned int bit_l, const char* texts_p, unsigned int texts_l);

/* flowlike_http */
__attribute__((import_module("flowlike_http"), import_name("request")))
int fl_http_request(int method, const char* url_p, unsigned int url_l, const char* hdr_p, unsigned int hdr_l, const char* body_p, unsigned int body_l);

/* flowlike_stream */
__attribute__((import_module("flowlike_stream"), import_name("emit")))
void fl_stream_emit(const char* evt_p, unsigned int evt_l, const char* data_p, unsigned int data_l);
__attribute__((import_module("flowlike_stream"), import_name("text")))
void fl_stream_text(const char* txt_p, unsigned int txt_l);

/* flowlike_auth */
__attribute__((import_module("flowlike_auth"), import_name("get_oauth_token")))
long long fl_get_oauth_token(const char* prov_p, unsigned int prov_l);
__attribute__((import_module("flowlike_auth"), import_name("has_oauth_token")))
int fl_has_oauth_token(const char* prov_p, unsigned int prov_l);
