// App commands are declared here so each one needs an explicit permission
// in capabilities/default.json; anything not listed there is unreachable from the UI.
const COMMANDS: &[&str] = &[
    "app_status",
    "unlock",
    "reset_data",
    "list_roots",
    "add_root",
    "update_root",
    "remove_root",
    "get_excludes",
    "set_excludes",
    "start_scan",
    "cancel_job",
    "list_repos",
    "repo_detail",
    "refresh_repos",
    "get_graph",
    "commit_files",
    "list_accounts",
    "save_account",
    "delete_account",
    "link_account",
    "unlink_account",
    "preview_switch",
    "apply_switch",
    "preview_bulk_switch",
    "apply_bulk_switch",
    "push_check",
    "probe_clone_access",
    "start_clone",
    "start_fetch",
    "start_access_scan",
    "access_matrix",
    "open_in_editor",
    "open_on_github",
    "reveal_repo",
    "generate_ssh_key",
    "test_ssh_key",
    "open_ssh_key_page",
    "trust_github_host",
    "repair_account_key",
    "list_ssh_keys",
    "get_settings",
    "set_settings",
    "clear_graph_cache",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri build script");
}
