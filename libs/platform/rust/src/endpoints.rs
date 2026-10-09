use crate::{Client, Method, RequestOptions, Result, Value};

impl Client {
    pub async fn apply_table_action(
        &self,
        app_id: &str,
        table: &str,
        selector: &crate::DatabaseSelector,
        action: &crate::DatabaseAction,
    ) -> Result<Value> {
        self.table_reference_action(
            app_id,
            table,
            &selector.options().json(serde_json::to_value(action)?),
        )
        .await
    }
}

// Keep route mappings beside their public names. Dynamic IDs are encoded by Client::url.
macro_rules! endpoints {
    ($( $name:ident ( $($arg:ident),* ) $method:ident [$($segment:expr),*]; )*) => {
        impl Client {
            $(pub async fn $name(&self, $($arg: &str,)* options: &RequestOptions) -> Result<Value> {
                self.request(Method::$method, &[$($segment),*], options).await
            })*
        }
    };
}

endpoints! {
    health() GET ["health"];
    list_apps() GET ["apps"];
    search_apps() GET ["apps", "search"];
    get_app(app_id) GET ["apps", app_id];
    create_app() PUT ["apps", "new"];
    update_app(app_id) PUT ["apps", app_id];
    delete_app(app_id) DELETE ["apps", app_id];
    get_app_detail(app_id) GET ["apps", app_id, "detail"];
    get_app_meta(app_id) GET ["apps", app_id, "meta"];
    update_app_meta(app_id) PUT ["apps", app_id, "meta"];
    change_app_visibility(app_id) PATCH ["apps", app_id, "visibility"];
    fork_app(app_id) POST ["apps", app_id, "fork"];
    get_fork_preview(app_id) GET ["apps", app_id, "fork", "preview"];
    get_fork_job(job_id) GET ["apps", "fork", "jobs", job_id];
    list_publication_requests(app_id) GET ["apps", app_id, "publication"];
    request_publication(app_id) POST ["apps", app_id, "publication", "request"];
    list_nodes() GET ["apps", "nodes"];
    list_app_nodes(app_id) GET ["apps", app_id, "nodes"];
    search_templates() GET ["apps", "templates", "search"];

    list_boards(app_id) GET ["apps", app_id, "board"];
    get_board(app_id, board_id) GET ["apps", app_id, "board", board_id];
    upsert_board(app_id, board_id) PUT ["apps", app_id, "board", board_id];
    delete_board(app_id, board_id) DELETE ["apps", app_id, "board", board_id];
    version_board(app_id, board_id) PATCH ["apps", app_id, "board", board_id];
    execute_commands(app_id, board_id) POST ["apps", app_id, "board", board_id];
    get_board_versions(app_id, board_id) GET ["apps", app_id, "board", board_id, "version"];
    get_board_version_infos(app_id, board_id) GET ["apps", app_id, "board", board_id, "version", "info"];
    get_board_version_current(app_id, board_id) GET ["apps", app_id, "board", board_id, "version", "current"];
    publish_board_if_changed(app_id, board_id) POST ["apps", app_id, "board", board_id, "version", "current"];
    sync_board(app_id, board_id) POST ["apps", app_id, "board", board_id, "sync"];
    undo_board(app_id, board_id) PATCH ["apps", app_id, "board", board_id, "undo"];
    redo_board(app_id, board_id) PATCH ["apps", app_id, "board", board_id, "redo"];
    prerun_board(app_id, board_id) GET ["apps", app_id, "board", board_id, "prerun"];
    get_board_workspace(app_id, board_id) GET ["apps", app_id, "board", board_id, "workspace"];
    get_board_capabilities(app_id) GET ["apps", app_id, "board", "capabilities"];
    get_board_summaries(app_id) GET ["apps", app_id, "board", "summaries"];
    get_board_variables(app_id) GET ["apps", app_id, "board", "variables"];
    get_flowscript(app_id, board_id) GET ["apps", app_id, "board", board_id, "flowscript"];
    apply_flowscript(app_id, board_id) POST ["apps", app_id, "board", board_id, "flowscript", "apply"];
    render_flowscript(app_id, board_id) POST ["apps", app_id, "board", board_id, "flowscript", "render"];
    format_flowscript(app_id, board_id) POST ["apps", app_id, "board", board_id, "flowscript", "format"];
    get_flow_ir_commit_disposition(app_id, board_id) POST ["apps", app_id, "board", board_id, "flow-ir-commit", "disposition"];
    apply_flow_ir_commit(app_id, board_id) POST ["apps", app_id, "board", board_id, "flow-ir-commit", "apply"];
    list_runs(app_id, board_id) GET ["apps", app_id, "board", board_id, "runs"];
    get_run_logs(app_id, board_id) POST ["apps", app_id, "board", board_id, "logs", "query"];
    count_run_logs(app_id, board_id) POST ["apps", app_id, "board", board_id, "logs", "count"];
    get_run_summary(app_id, board_id) GET ["apps", app_id, "board", board_id, "logs", "summary"];
    get_run_payload(app_id, board_id, run_id) GET ["apps", app_id, "board", board_id, "runs", run_id, "payload"];
    get_execution_elements(app_id, board_id) GET ["apps", app_id, "board", board_id, "elements"];
    get_element_demand(app_id, board_id) GET ["apps", app_id, "board", board_id, "element-demand"];

    list_events(app_id) GET ["apps", app_id, "events"];
    get_event(app_id, event_id) GET ["apps", app_id, "events", event_id];
    upsert_event(app_id, event_id) PUT ["apps", app_id, "events", event_id];
    delete_event(app_id, event_id) DELETE ["apps", app_id, "events", event_id];
    get_event_versions(app_id, event_id) GET ["apps", app_id, "events", event_id, "versions"];
    get_event_timeline(app_id, event_id) GET ["apps", app_id, "events", event_id, "timeline"];
    get_event_runs(app_id, event_id) GET ["apps", app_id, "events", event_id, "runs"];
    restore_event(app_id, event_id) POST ["apps", app_id, "events", event_id, "restore"];
    validate_event(app_id, event_id) POST ["apps", app_id, "events", event_id, "validate"];
    setup_event(app_id, event_id) POST ["apps", app_id, "events", event_id, "setup"];
    list_event_setups(app_id, event_id) GET ["apps", app_id, "events", event_id, "setups"];
    prerun_event(app_id, event_id) GET ["apps", app_id, "events", event_id, "prerun"];
    get_event_registrations(app_id, event_id) GET ["apps", app_id, "events", event_id, "registrations"];
    list_schedules() GET ["user", "schedules"];
    explain_event_canary(app_id, event_id) GET ["apps", app_id, "events", event_id, "canary", "explain"];
    get_event_canary_stats(app_id, event_id) GET ["apps", app_id, "events", event_id, "canary", "stats"];
    update_event_canary(app_id, event_id) PATCH ["apps", app_id, "events", event_id, "canary"];
    promote_event_canary(app_id, event_id) POST ["apps", app_id, "events", event_id, "canary", "promote"];
    abort_event_canary(app_id, event_id) POST ["apps", app_id, "events", event_id, "canary", "abort"];
    update_event_variants(app_id, event_id) PUT ["apps", app_id, "events", event_id, "variants"];
    list_event_aliases(app_id, event_id) GET ["apps", app_id, "events", event_id, "alias"];
    upsert_event_alias(app_id, event_id, slug) PUT ["apps", app_id, "events", event_id, "alias", slug];
    delete_event_alias(app_id, event_id, slug) DELETE ["apps", app_id, "events", event_id, "alias", slug];

    list_tables(app_id) GET ["apps", app_id, "db"];
    list_user_tables(app_id) GET ["apps", app_id, "db", "user"];
    create_table(app_id, table) POST ["apps", app_id, "db", table];
    drop_table(app_id, table) DELETE ["apps", app_id, "db", table, "table"];
    add_to_table(app_id, table) PUT ["apps", app_id, "db", table];
    delete_from_table(app_id, table) DELETE ["apps", app_id, "db", table];
    list_table_items(app_id, table) GET ["apps", app_id, "db", table];
    query_table(app_id, table) POST ["apps", app_id, "db", table, "query"];
    get_table_schema(app_id, table) GET ["apps", app_id, "db", table, "schema"];
    count_table(app_id, table) GET ["apps", app_id, "db", table, "count"];
    update_table(app_id, table) PUT ["apps", app_id, "db", table, "update"];
    add_table_columns(app_id, table) POST ["apps", app_id, "db", table, "columns"];
    alter_table_columns(app_id, table) PUT ["apps", app_id, "db", table, "columns"];
    drop_table_columns(app_id, table) DELETE ["apps", app_id, "db", table, "columns"];
    set_table_primary_key(app_id, table) PUT ["apps", app_id, "db", table, "primary-key"];
    build_table_index(app_id, table) POST ["apps", app_id, "db", table, "index"];
    drop_table_index(app_id, table, index_name) DELETE ["apps", app_id, "db", table, "index", index_name];
    get_table_indices(app_id, table) GET ["apps", app_id, "db", table, "indices"];
    get_table_view(app_id, table) GET ["apps", app_id, "db", table, "view"];
    optimize_table(app_id, table) POST ["apps", app_id, "db", table, "optimize"];
    get_table_history(app_id, table) GET ["apps", app_id, "db", table, "references"];
    table_reference_action(app_id, table) POST ["apps", app_id, "db", table, "references"];
    compare_table_versions(app_id, table) POST ["apps", app_id, "db", table, "compare"];
    presign_database_access(app_id) POST ["apps", app_id, "db", "presign"];
    presign_project_database_access(app_id) POST ["apps", app_id, "db", "presign", "project"];
    list_saved_queries(app_id) GET ["apps", app_id, "db", "queries"];
    create_saved_query(app_id) POST ["apps", app_id, "db", "queries"];
    execute_saved_query(app_id) POST ["apps", app_id, "db", "queries", "execute"];
    get_saved_query(app_id, query_id) GET ["apps", app_id, "db", "queries", query_id];
    update_saved_query(app_id, query_id) PUT ["apps", app_id, "db", "queries", query_id];
    delete_saved_query(app_id, query_id) DELETE ["apps", app_id, "db", "queries", query_id];

    list_pages(app_id) GET ["apps", app_id, "pages"];
    get_page(app_id, page_id) GET ["apps", app_id, "pages", page_id];
    upsert_page(app_id, page_id) PUT ["apps", app_id, "pages", page_id];
    delete_page(app_id, page_id) DELETE ["apps", app_id, "pages", page_id];
    get_page_by_route(app_id) GET ["apps", app_id, "pages", "by-route"];
    get_page_bootstrap(app_id) GET ["apps", app_id, "pages", "bootstrap"];
    list_routes(app_id) GET ["apps", app_id, "routes"];
    create_route(app_id) POST ["apps", app_id, "routes"];
    update_route(app_id, route_id) PUT ["apps", app_id, "routes", route_id];
    delete_route(app_id, route_id) DELETE ["apps", app_id, "routes", route_id];
    get_route_by_path(app_id) GET ["apps", app_id, "routes", "by-path"];
    get_default_route(app_id) GET ["apps", app_id, "routes", "default"];
    list_widgets(app_id) GET ["apps", app_id, "widgets"];
    get_widget(app_id, widget_id) GET ["apps", app_id, "widgets", widget_id];
    upsert_widget(app_id, widget_id) PUT ["apps", app_id, "widgets", widget_id];
    delete_widget(app_id, widget_id) DELETE ["apps", app_id, "widgets", widget_id];
    get_widget_versions(app_id, widget_id) GET ["apps", app_id, "widgets", widget_id, "versions"];
    create_widget_version(app_id, widget_id) POST ["apps", app_id, "widgets", widget_id, "versions"];
    list_connections(app_id) GET ["apps", app_id, "connections"];
    add_connection(app_id) POST ["apps", app_id, "connections"];
    update_connection(app_id, connection_id) PUT ["apps", app_id, "connections", connection_id];
    remove_connection(app_id, connection_id) DELETE ["apps", app_id, "connections", connection_id];
    request_connection(app_id) PUT ["apps", app_id, "connections", "request"];
    accept_connection_request(app_id, connection_id) POST ["apps", app_id, "connections", "queue", connection_id];
    reject_connection_request(app_id, connection_id) DELETE ["apps", app_id, "connections", "queue", connection_id];
    get_accessible_apps(app_id) GET ["apps", app_id, "connections", "accessible"];
    get_connection_graph(app_id) GET ["apps", app_id, "connections", "graph"];
    get_connection_tables(app_id, target_app_id) GET ["apps", app_id, "connections", target_app_id, "tables"];
    get_connection_events(app_id, target_app_id) GET ["apps", app_id, "connections", target_app_id, "events"];
    list_packages(app_id) GET ["apps", app_id, "packages"];
    add_package(app_id) POST ["apps", app_id, "packages"];
    update_package(app_id, package_id) PATCH ["apps", app_id, "packages", package_id];
    remove_package(app_id, package_id) DELETE ["apps", app_id, "packages", package_id];
    check_package_updates(app_id) GET ["apps", app_id, "packages", "updates"];
    reactivate_package(app_id, package_id) POST ["apps", app_id, "packages", package_id, "reactivate"];
    get_package_patch_info(app_id, package_id) GET ["apps", app_id, "packages", package_id, "patch-info"];
    list_roles(app_id) GET ["apps", app_id, "roles"];
    get_own_role(app_id) GET ["apps", app_id, "roles", "me"];
    upsert_role(app_id, role_id) PUT ["apps", app_id, "roles", role_id];
    delete_role(app_id, role_id) DELETE ["apps", app_id, "roles", role_id];
    make_role_default(app_id, role_id) PUT ["apps", app_id, "roles", role_id, "default"];
    assign_role(app_id, role_id, sub) POST ["apps", app_id, "roles", role_id, "assign", sub];
    get_team(app_id) GET ["apps", app_id, "team"];
    invite_user(app_id) PUT ["apps", app_id, "team", "invite"];
    remove_team_user(app_id, sub) DELETE ["apps", app_id, "team", sub];
    list_invites(app_id) GET ["apps", app_id, "team", "invites"];
    revoke_invite(app_id, invite_id) DELETE ["apps", app_id, "team", "invites", invite_id];
    create_invite_link(app_id) PUT ["apps", app_id, "team", "link"];
    list_invite_links(app_id) GET ["apps", app_id, "team", "link"];
    remove_invite_link(app_id, link_id) DELETE ["apps", app_id, "team", "link", link_id];
    join_invite_link(app_id, token) POST ["apps", app_id, "team", "link", "join", token];
    list_join_requests(app_id) GET ["apps", app_id, "team", "queue"];
    request_join(app_id) PUT ["apps", app_id, "team", "queue"];
    accept_join_request(app_id, request_id) POST ["apps", app_id, "team", "queue", request_id];
    reject_join_request(app_id, request_id) DELETE ["apps", app_id, "team", "queue", request_id];
    list_api_keys(app_id) GET ["apps", app_id, "api"];
    create_api_key(app_id) PUT ["apps", app_id, "api"];
    delete_api_key(app_id, key_id) DELETE ["apps", app_id, "api", key_id];

    list_devices() GET ["devices"];
    get_device(device_id) GET ["devices", device_id];
    rename_device(device_id) PATCH ["devices", device_id];
    revoke_device(device_id) DELETE ["devices", device_id];
    get_device_setup() GET ["devices", "setup"];
    get_device_usage() GET ["devices", "usage"];
    list_device_enrollments() GET ["devices", "enrollments"];
    create_device_enrollment() POST ["devices", "enrollments"];
    cancel_device_enrollment(enrollment_id) DELETE ["devices", "enrollments", enrollment_id];
    get_device_management_policy(device_id) GET ["devices", device_id, "management", "policy"];
    update_device_management_policy(device_id) PUT ["devices", device_id, "management", "policy"];
    get_device_access(device_id) GET ["devices", device_id, "management", "my-access"];
    get_device_identity(device_id) GET ["devices", device_id, "identity"];
    signal_device_controller(device_id) POST ["devices", device_id, "signaling", "controller"];
    get_device_inventory(device_id, key) GET ["devices", device_id, "inventory", key];
    update_device_inventory(device_id, key) PUT ["devices", device_id, "inventory", key];
    list_controller_vaults() GET ["devices", "controller-vaults"];
    get_controller_vault(vault_id) GET ["devices", "controller-vaults", vault_id];
    update_controller_vault(vault_id) PUT ["devices", "controller-vaults", vault_id];
    get_device_resource_summary() GET ["devices", "resource-summary"];
    list_device_resource_grants(device_id) GET ["devices", device_id, "resource-grants"];
    create_device_resource_grant(device_id) POST ["devices", device_id, "resource-grants"];
    get_device_resource_grant(device_id, grant_id) GET ["devices", device_id, "resource-grants", grant_id];
    revoke_device_resource_grant(device_id, grant_id) DELETE ["devices", device_id, "resource-grants", grant_id];
    get_device_grant_billing(device_id, grant_id) GET ["devices", device_id, "resource-grants", grant_id, "billing"];
    approve_device_grant_billing(device_id, grant_id) POST ["devices", device_id, "resource-grants", grant_id, "billing"];
    get_device_billing_eligibility(device_id, grant_id) GET ["devices", device_id, "resource-grants", grant_id, "billing", "eligibility"];
    list_device_billing_grants(device_id) GET ["devices", device_id, "billing-grants"];
    get_device_billing_grant(device_id, billing_id) GET ["devices", device_id, "billing-grants", billing_id];
    revoke_device_billing_grant(device_id, billing_id) DELETE ["devices", device_id, "billing-grants", billing_id];
    get_device_billing_usage(device_id, billing_id) GET ["devices", device_id, "billing-grants", billing_id, "usage"];
    list_device_instances(device_id) GET ["devices", device_id, "instances"];
    register_device_instance(device_id) POST ["devices", device_id, "instances"];
    retire_device_instance(device_id, instance_id) DELETE ["devices", device_id, "instances", instance_id];
    get_app_device_metadata(app_id) GET ["apps", app_id, "device-metadata"];
    get_app_device_placements(app_id) GET ["apps", app_id, "device-placements"];
    release_device_schedule(app_id, event_id) PUT ["apps", app_id, "device-schedules", event_id];
    return_device_schedule(app_id, event_id) DELETE ["apps", app_id, "device-schedules", event_id];

    search_bits() POST ["bit"];
    get_bit(bit_id) GET ["bit", bit_id];
    get_bit_dependencies(bit_id) GET ["bit", bit_id, "dependencies"];
    chat_completions() POST ["chat", "completions"];
    responses() POST ["responses"];
    embed() POST ["embeddings", "embed"];
    get_usage() GET ["chat", "usage"];
}
