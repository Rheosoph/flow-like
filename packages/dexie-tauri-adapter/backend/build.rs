const COMMANDS: &[&str] = &[
    "blob_store",
    "blob_get",
    "blob_store_batch",
    "blob_get_batch",
    "blob_delete",
    "blob_configure",
    "blob_inc_refs",
    "blob_dec_refs",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
