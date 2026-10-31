mod accounts;
mod commands;
mod core;
mod db;
mod error;
mod git;
mod graph;
mod inspect;
mod remote_url;
mod scanner;
mod sshkeys;
mod store;

use tauri::Manager;

/// GUI launches on macOS and Linux get a minimal PATH; borrow the login shell's
/// so the user's own git, ssh and editor CLI are found.
#[cfg(unix)]
fn inherit_shell_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut c = std::process::Command::new(shell);
    c.args(["-ilc", "printf '%s' \"$PATH\""]);
    if let Ok(o) = git::run_limited(c, std::time::Duration::from_secs(3), None) {
        let p = o.stdout.lines().last().unwrap_or("").trim();
        if o.ok() && !p.is_empty() {
            // SAFETY: runs at startup before any other thread exists.
            unsafe { std::env::set_var("PATH", p) };
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(unix)]
    inherit_shell_path();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let core = core::Core::new(app.handle().clone(), dir);
            core.try_open_with_keychain();
            core.start_background();
            app.manage(core);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(f) = event
                && let Some(core) = window.try_state::<std::sync::Arc<core::Core>>()
            {
                core.on_focus(*f);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::unlock,
            commands::reset_data,
            commands::list_roots,
            commands::add_root,
            commands::update_root,
            commands::remove_root,
            commands::get_excludes,
            commands::set_excludes,
            commands::start_scan,
            commands::cancel_job,
            commands::list_repos,
            commands::repo_detail,
            commands::refresh_repos,
            commands::get_graph,
            commands::commit_files,
            commands::list_accounts,
            commands::save_account,
            commands::delete_account,
            commands::link_account,
            commands::unlink_account,
            commands::preview_switch,
            commands::apply_switch,
            commands::preview_bulk_switch,
            commands::apply_bulk_switch,
            commands::push_check,
            commands::probe_clone_access,
            commands::start_clone,
            commands::start_fetch,
            commands::start_access_scan,
            commands::access_matrix,
            commands::open_in_editor,
            commands::open_on_github,
            commands::reveal_repo,
            commands::generate_ssh_key,
            commands::test_ssh_key,
            commands::open_ssh_key_page,
            commands::trust_github_host,
            commands::repair_account_key,
            commands::list_ssh_keys,
            commands::get_settings,
            commands::set_settings,
            commands::clear_graph_cache,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RepoDeck");
}
