// Typed wrappers around the Rust core's allow-listed commands.
import { invoke } from "@tauri-apps/api/core";

export type DbState =
  | "ready"
  | { needs_passphrase: { first_time: boolean } }
  | "key_lost"
  | { error: { message: string } };

export interface AppStatus {
  db: DbState;
  git_ok: boolean;
  git_version: string | null;
  git_too_old: boolean;
  onboarded: boolean;
  platform: string;
}

export interface ScanRoot {
  id: number;
  path: string;
  max_depth: number;
  follow_symlinks: boolean;
  stop_at_repo: boolean;
  enabled: boolean;
  last_scanned_at: number | null;
}

export interface RemoteInfo {
  name: string;
  url: string;
  host: string | null;
  owner: string | null;
  repo: string | null;
  is_github: boolean;
  web_url: string | null;
}

export interface Fix {
  kind: "switch" | "link_switch" | "edit_account" | "repair_key" | "trust_host" | "add_account";
  account_id: number | null;
  label: string;
}

export interface Issue {
  level: "warn" | "error";
  text: string;
  fix: Fix | null;
}

export interface Health {
  level: "ok" | "warn" | "error" | "unknown";
  kind: string;
  text: string;
  username: string | null;
  checked_at: number | null;
  fix_path: string | null;
}

export type RepoKind = "normal" | "submodule" | "worktree" | "nested" | "bare";

export interface RepoRow {
  id: number;
  path: string;
  name: string;
  kind: RepoKind;
  parent_repo_id: number | null;
  parent_name: string | null;
  root_id: number | null;
  is_bare: boolean;
  head_branch: string | null;
  is_dirty: boolean | null;
  last_commit_at: number | null;
  effective_name: string | null;
  effective_email: string | null;
  email_scope: string | null;
  active_account_id: number | null;
  linked_account_ids: number[];
  remote: RemoteInfo | null;
  last_fetch_at: number | null;
  last_fetch_result: string | null;
  inspected: boolean;
  status: "ok" | "warn" | "error" | "pending";
  issues: Issue[];
  suggested_account_id: number | null;
}

export interface ConfigEntry {
  key: string;
  value: string;
  scope: string;
  origin_file: string;
}

export interface Branch {
  name: string;
  is_head: boolean;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
  gone: boolean;
  last_commit_at: number | null;
}

export interface PushCheck {
  account_id: number;
  status: CheckStatus;
  detail: string | null;
  checked_at: number;
}

export type CheckStatus =
  | "can_push"
  | "can_read"
  | "read_only"
  | "no_access"
  | "wrong_account"
  | "auth_failed"
  | "host_untrusted"
  | "offline"
  | "unknown";

export interface RepoDetail {
  row: RepoRow;
  config: ConfigEntry[];
  remotes: RemoteInfo[];
  branches: Branch[];
  push_checks: PushCheck[];
  authors: [string, number][];
  inspect_error: string | null;
}

export interface Account {
  id: number | null;
  label: string;
  user_name: string;
  user_email: string;
  host: string;
  auth_method: "ssh" | "https";
  ssh_key_path: string | null;
  expected_username: string | null;
  owners: string | null;
  /** Computed by the core; not sent back when saving. */
  health?: Health | null;
}

export interface ConfigChange {
  key: string;
  current: string | null;
  next: string | null;
}

export interface Preview {
  changes: ConfigChange[];
  commands: string[];
  config_file: string;
}

export interface Seg {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  c: number;
}

export interface GraphRow {
  hash: string;
  parents: string[];
  author_name: string;
  author_email: string;
  authored_at: number;
  subject: string;
  refs: string;
  lane: number;
  segments: Seg[];
}

export interface GraphPage {
  rows: GraphRow[];
  offset: number;
  has_more: boolean;
}

export interface ChangedFile {
  status: string;
  path: string;
}

export interface CheckResult {
  status: CheckStatus;
  detail: string;
  username: string | null;
}

export interface MatrixCell {
  repo_id: number;
  account_id: number;
  status: CheckStatus;
  detail: string | null;
  checked_at: number;
}

export interface JobEvent {
  id: number;
  kind: "scan" | "fetch" | "access" | "clone";
  done: number;
  total: number;
  label: string;
  finished: boolean;
  cancelled: boolean;
  summary: Record<string, number>;
}

export interface JobItem {
  job_id: number;
  kind: "fetch" | "access" | "clone";
  repo_id: number;
  account_id: number | null;
  status: string;
  detail: string;
}

export interface CloneAccess {
  account_id: number;
  status: CheckStatus;
  detail: string;
}

export interface BulkItem {
  repo_id: number;
  name: string;
  changes: ConfigChange[];
}

export interface BulkResult {
  applied: number;
  /** [repo name, reason] */
  skipped: [string, string][];
}

export interface SshKeyInfo {
  path: string;
  name: string;
  fingerprint: string | null;
  comment: string | null;
  kind: string | null;
  has_public: boolean;
  in_agent: boolean;
  used_by: string[];
}

/** File-name part RepoDeck uses for a generated key. Must match `sshkeys::slug` in Rust. */
export function keySlug(label: string): string {
  const s = label
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return s || "repodeck";
}

export interface GeneratedKey {
  private_path: string;
  public_key: string;
  fingerprint: string;
}

export type Settings = Record<string, string>;

export const api = {
  appStatus: () => invoke<AppStatus>("app_status"),
  unlock: (passphrase: string) => invoke<void>("unlock", { passphrase }),
  resetData: () => invoke<void>("reset_data"),

  listRoots: () => invoke<ScanRoot[]>("list_roots"),
  addRoot: (path: string, stopAtRepo = false) => invoke<number>("add_root", { path, stopAtRepo }),
  updateRoot: (root: ScanRoot) => invoke<void>("update_root", { root }),
  removeRoot: (id: number) => invoke<void>("remove_root", { id }),
  getExcludes: () => invoke<string[]>("get_excludes"),
  setExcludes: (globs: string[]) => invoke<void>("set_excludes", { globs }),
  startScan: (rootIds?: number[]) => invoke<number>("start_scan", { rootIds: rootIds ?? null }),
  cancelJob: (id: number) => invoke<void>("cancel_job", { id }),

  listRepos: () => invoke<RepoRow[]>("list_repos"),
  repoDetail: (id: number) => invoke<RepoDetail>("repo_detail", { id }),
  refreshRepos: (ids?: number[], force = false) =>
    invoke<number>("refresh_repos", { ids: ids ?? null, force }),
  getGraph: (id: number, page: number) => invoke<GraphPage>("get_graph", { id, page }),
  commitFiles: (id: number, hash: string, parent: string | null) =>
    invoke<ChangedFile[]>("commit_files", { id, hash, parent }),

  listAccounts: () => invoke<Account[]>("list_accounts"),
  saveAccount: (account: Account) => invoke<number>("save_account", { account }),
  deleteAccount: (id: number) => invoke<void>("delete_account", { id }),
  linkAccount: (repoIds: number[], accountId: number) =>
    invoke<void>("link_account", { repoIds, accountId }),
  unlinkAccount: (repoId: number, accountId: number) =>
    invoke<void>("unlink_account", { repoId, accountId }),
  previewSwitch: (repoId: number, accountId: number | null) =>
    invoke<Preview>("preview_switch", { repoId, accountId }),
  applySwitch: (repoId: number, accountId: number | null, expected: ConfigChange[]) =>
    invoke<RepoRow>("apply_switch", { repoId, accountId, expected }),
  previewBulkSwitch: (repoIds: number[], accountId: number | null) =>
    invoke<BulkItem[]>("preview_bulk_switch", { repoIds, accountId }),
  applyBulkSwitch: (accountId: number | null, expected: BulkItem[]) =>
    invoke<BulkResult>("apply_bulk_switch", { accountId, expected }),
  pushCheck: (repoId: number, accountId: number) =>
    invoke<CheckResult>("push_check", { repoId, accountId }),
  generateSshKey: (label: string, email: string) => invoke<GeneratedKey>("generate_ssh_key", { label, email }),
  testSshKey: (keyPath: string, host: string, expectedUsername: string | null, accountId: number | null = null) =>
    invoke<CheckResult>("test_ssh_key", { keyPath, host, expectedUsername, accountId }),
  listSshKeys: () => invoke<SshKeyInfo[]>("list_ssh_keys"),
  repairAccountKey: (accountId: number) => invoke<number>("repair_account_key", { accountId }),
  openSshKeyPage: (host: string) => invoke<void>("open_ssh_key_page", { host }),
  trustGithubHost: () => invoke<void>("trust_github_host"),
  probeCloneAccess: (url: string) => invoke<CloneAccess[]>("probe_clone_access", { url }),
  startClone: (url: string, dest: string, accountId: number) =>
    invoke<number>("start_clone", { url, dest, accountId }),
  startFetch: (repoIds: number[]) => invoke<number>("start_fetch", { repoIds }),
  startAccessScan: (changedOnly: boolean, repoIds?: number[]) =>
    invoke<number>("start_access_scan", { changedOnly, repoIds: repoIds ?? null }),
  accessMatrix: () => invoke<MatrixCell[]>("access_matrix"),

  openInEditor: (repoId: number) => invoke<void>("open_in_editor", { repoId }),
  openOnGithub: (repoId: number, branch?: string, commit?: string) =>
    invoke<void>("open_on_github", { repoId, branch: branch ?? null, commit: commit ?? null }),
  revealRepo: (repoId: number) => invoke<void>("reveal_repo", { repoId }),

  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (values: Settings) => invoke<void>("set_settings", { values }),
  clearGraphCache: () => invoke<void>("clear_graph_cache"),
};

export const checkWords: Record<CheckStatus, string> = {
  can_push: "Can push",
  can_read: "Can read",
  read_only: "Read only",
  no_access: "No access",
  wrong_account: "Wrong account",
  auth_failed: "Sign-in failed",
  host_untrusted: "Host not trusted",
  offline: "Offline",
  unknown: "Unknown",
};

export const scopeWords: Record<string, string> = {
  local: "this repo's config",
  worktree: "this worktree's config",
  includeIf: "an includeIf rule",
  global: "your global ~/.gitconfig",
  system: "the system Git config",
  command: "the command line",
};

export function ago(ts: number | null | undefined): string {
  if (!ts) return "—";
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)}d ago`;
  if (s < 86400 * 365) return `${Math.floor(s / (86400 * 30))}mo ago`;
  return `${Math.floor(s / (86400 * 365))}y ago`;
}

export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
