// App-wide reactive state. Components read from here; the Rust core pushes updates as events.
import { listen } from "@tauri-apps/api/event";
import { SvelteSet } from "svelte/reactivity";
import {
  api,
  errText,
  type Account,
  type Fix,
  type JobEvent,
  type JobItem,
  type RepoRow,
  type ScanRoot,
  type Settings,
} from "./api";

export type View = "repos" | "accounts" | "settings";
export type GroupBy = "none" | "owner" | "account" | "folder";
export type Entry = { kind: "repo"; repo: RepoRow; depth: number };
export type Header = { kind: "header"; key: string; count: number; problems: number; collapsed: boolean };
export type ListRow = Entry | Header;

export interface JobState extends JobEvent {
  failed: number[];
}

class AppState {
  view = $state<View>("repos");
  repos = $state<RepoRow[]>([]);
  accounts = $state<Account[]>([]);
  roots = $state<ScanRoot[]>([]);
  settings = $state<Settings>({});
  selectedId = $state<number | null>(null);
  detailOpen = $state(false);
  jobs = $state<JobState[]>([]);
  toast = $state<{ text: string; error: boolean } | null>(null);
  cloneOpen = $state(false);
  /** Account editor to open on the Accounts screen ("new" = add). */
  editAccountId = $state<number | "new" | null>(null);
  /** Repos picked for a bulk action (⌘/Ctrl-click, Shift-click). */
  selectedIds = new SvelteSet<number>();
  /** Pending bulk switch; BulkSwitchDialog previews and confirms it. */
  bulkReq = $state<{ repoIds: number[]; accountId: number | null } | null>(null);
  /** Suggestion strips the user dismissed, by account id (remembered on this device). */
  dismissed = $state<number[]>(readDismissed());
  /** Pending account switch; the preview dialog confirms it. accountId null = use global. */
  switchReq = $state<{ repoId: number; accountId: number | null } | null>(null);

  // Filters
  query = $state("");
  rootFilter = $state<number | "all">("all");
  accountFilter = $state<number | "all" | "none">("all");
  attentionOnly = $state(false);
  groupBy = $state<GroupBy>(readGroupBy());
  collapsed = new SvelteSet<string>();

  accountById = $derived(new Map(this.accounts.map((a) => [a.id!, a])));
  repoById = $derived(new Map(this.repos.map((r) => [r.id, r])));
  selected = $derived(this.selectedId == null ? null : (this.repoById.get(this.selectedId) ?? null));
  /** Accounts that can't sign in (missing key, public key, failed test). */
  unhealthyAccounts = $derived(this.accounts.filter((a) => a.health?.level === "error"));
  /** Repos that look like they belong to an account other than the active one, grouped by account. */
  suggestions = $derived.by(() => {
    const by = new Map<number, number[]>();
    for (const r of this.repos) {
      const s = r.suggested_account_id;
      if (s == null || r.is_bare || !r.inspected || s === r.active_account_id) continue;
      if (this.dismissed.includes(s) || !this.accountById.has(s)) continue;
      by.set(s, [...(by.get(s) ?? []), r.id]);
    }
    return [...by].map(([accountId, repoIds]) => ({ accountId, repoIds }));
  });
  attentionCount = $derived(this.repos.filter((r) => r.status === "warn" || r.status === "error").length);

  /** Filtered list rows in display order: children (submodules, worktrees) right under their
   * parent, and group headers when grouping is on. */
  listRows = $derived.by((): ListRow[] => {
    const q = this.query.trim().toLowerCase();
    const match = (r: RepoRow) => {
      if (this.rootFilter !== "all" && r.root_id !== this.rootFilter) return false;
      if (this.accountFilter === "none" && r.active_account_id != null) return false;
      if (typeof this.accountFilter === "number" && !r.linked_account_ids.includes(this.accountFilter))
        return false;
      if (this.attentionOnly && r.status !== "warn" && r.status !== "error") return false;
      if (!q) return true;
      return (
        r.name.toLowerCase().includes(q) ||
        r.path.toLowerCase().includes(q) ||
        (r.effective_email ?? "").toLowerCase().includes(q) ||
        (r.remote?.owner ?? "").toLowerCase().includes(q)
      );
    };
    const isChild = (r: RepoRow) =>
      (r.kind === "submodule" || r.kind === "worktree") &&
      r.parent_repo_id != null &&
      this.repoById.has(r.parent_repo_id);
    const children = new Map<number, RepoRow[]>();
    for (const r of this.repos) {
      if (isChild(r)) {
        const list = children.get(r.parent_repo_id!) ?? [];
        list.push(r);
        children.set(r.parent_repo_id!, list);
      }
    }
    // A parent stays visible when any of its children match.
    const build = (r: RepoRow, depth: number): Entry[] => {
      const kidRows = (children.get(r.id) ?? []).flatMap((k) => build(k, depth + 1));
      return match(r) || kidRows.length ? [{ kind: "repo", repo: r, depth }, ...kidRows] : [];
    };
    const blocks = this.repos
      .filter((r) => !isChild(r))
      .map((r) => build(r, 0))
      .filter((b) => b.length);
    if (this.groupBy === "none") return blocks.flat();

    // Group whole blocks (a repo with its submodules/worktrees) by the top repo.
    const groups = new Map<string, Entry[]>();
    for (const b of blocks) {
      const key = this.groupKey(b[0].repo);
      groups.set(key, [...(groups.get(key) ?? []), ...b]);
    }
    const last = new Set(["No remote", "Global identity", "Other folders"]);
    const out: ListRow[] = [];
    for (const key of [...groups.keys()].sort((a, b) => Number(last.has(a)) - Number(last.has(b)) || a.localeCompare(b))) {
      const entries = groups.get(key)!;
      const problems = entries.filter((e) => e.repo.status === "warn" || e.repo.status === "error").length;
      const collapsed = this.collapsed.has(key);
      out.push({ kind: "header", key, count: entries.length, problems, collapsed });
      if (!collapsed) out.push(...entries);
    }
    return out;
  });

  /** Repo entries only, in display order (keyboard navigation, Fetch all, ranges). */
  visible = $derived(this.listRows.filter((r): r is Entry => r.kind === "repo"));

  groupKey(r: RepoRow): string {
    switch (this.groupBy) {
      case "owner":
        return r.remote?.owner ?? "No remote";
      case "account": {
        const a = r.active_account_id != null ? this.accountById.get(r.active_account_id) : null;
        return a?.label ?? "Global identity";
      }
      case "folder": {
        const root = this.roots.find((x) => x.id === r.root_id);
        return root ? root.path.split(/[\\/]/).pop() || root.path : "Other folders";
      }
      default:
        return "";
    }
  }

  setGroupBy(g: GroupBy) {
    this.groupBy = g;
    try {
      localStorage.setItem("repodeck.groupBy", g);
    } catch {
      // Not remembered this time; harmless.
    }
  }

  toggleGroup(key: string) {
    if (this.collapsed.has(key)) this.collapsed.delete(key);
    else this.collapsed.add(key);
  }

  async loadAll() {
    const [repos, accounts, roots, settings] = await Promise.all([
      api.listRepos(),
      api.listAccounts(),
      api.listRoots(),
      api.getSettings(),
    ]);
    this.repos = repos;
    this.accounts = accounts;
    this.roots = roots;
    this.settings = settings;
  }

  async reloadRepos() {
    this.repos = await api.listRepos();
  }

  async reloadAccounts() {
    this.accounts = await api.listAccounts();
  }

  async reloadRoots() {
    this.roots = await api.listRoots();
  }

  upsertRow(row: RepoRow) {
    const i = this.repos.findIndex((r) => r.id === row.id);
    if (i >= 0) this.repos[i] = row;
    else this.repos.push(row);
  }

  notify(text: string, error = false) {
    this.toast = { text, error };
    const t = this.toast;
    setTimeout(() => {
      if (this.toast === t) this.toast = null;
    }, error ? 6000 : 3000);
  }

  /** Run an action and show its error as a toast instead of throwing. */
  async run<T>(f: () => Promise<T>): Promise<T | undefined> {
    try {
      return await f();
    } catch (e) {
      this.notify(errText(e), true);
      return undefined;
    }
  }

  editAccount(id: number | "new") {
    this.view = "accounts";
    this.editAccountId = id;
  }

  /** Carry out the one-click fix offered next to a repo's issue. */
  async runFix(repoId: number, fix: Fix) {
    switch (fix.kind) {
      case "switch":
      case "link_switch":
        this.switchReq = { repoId, accountId: fix.account_id };
        return;
      case "edit_account":
        if (fix.account_id != null) this.editAccount(fix.account_id);
        return;
      case "add_account":
        this.editAccount("new");
        return;
      case "repair_key": {
        if (fix.account_id == null) return;
        const n = await this.run(() => api.repairAccountKey(fix.account_id!));
        if (n !== undefined) {
          await this.reloadAccounts();
          this.notify(`Key fixed${n ? `; updated ${n} repo${n === 1 ? "" : "s"}` : ""}`);
        }
        return;
      }
      case "trust_host": {
        const ok = await this.run(async () => {
          await api.trustGithubHost();
          return true;
        });
        if (!ok) return;
        if (fix.account_id != null) await this.run(() => api.pushCheck(repoId, fix.account_id!));
        this.notify("github.com is trusted now");
        return;
      }
    }
  }

  dismissSuggestion(accountId: number) {
    this.dismissed = [...this.dismissed, accountId];
    try {
      localStorage.setItem("repodeck.dismissedSuggestions", JSON.stringify(this.dismissed));
    } catch {
      // Storage can be unavailable; the strip just comes back next launch.
    }
  }

  dismissJob(id: number) {
    this.jobs = this.jobs.filter((j) => j.id !== id);
  }

  private reloadTimer: ReturnType<typeof setTimeout> | null = null;

  async listen() {
    await listen<RepoRow>("repo-updated", (e) => this.upsertRow(e.payload));
    await listen("repos-reload", () => {
      // Coalesce bursts from scans.
      if (this.reloadTimer) clearTimeout(this.reloadTimer);
      this.reloadTimer = setTimeout(() => {
        this.reloadRepos();
        this.reloadAccounts();
        this.reloadRoots();
      }, 150);
    });
    await listen<JobEvent>("job", (e) => {
      const j = e.payload;
      const i = this.jobs.findIndex((x) => x.id === j.id);
      if (i >= 0) this.jobs[i] = { ...this.jobs[i], ...j };
      else this.jobs.push({ ...j, failed: [] });
      if (j.finished && j.kind === "scan" && !j.cancelled) {
        setTimeout(() => this.dismissJob(j.id), 4000);
      }
    });
    await listen<JobItem>("job-item", (e) => {
      const it = e.payload;
      if (it.kind === "clone") {
        // Show the fresh clone once the list has it.
        this.view = "repos";
        this.selectedId = it.repo_id;
        this.detailOpen = true;
        return;
      }
      const bad =
        it.kind === "fetch"
          ? !["fetched", "no_remote", "cancelled"].includes(it.status)
          : false;
      if (!bad) return;
      const j = this.jobs.find((x) => x.id === it.job_id);
      if (j && !j.failed.includes(it.repo_id)) j.failed.push(it.repo_id);
    });
  }
}

function readGroupBy(): GroupBy {
  try {
    const v = localStorage.getItem("repodeck.groupBy");
    return v === "owner" || v === "account" || v === "folder" ? v : "none";
  } catch {
    return "none";
  }
}

function readDismissed(): number[] {
  try {
    const v = JSON.parse(localStorage.getItem("repodeck.dismissedSuggestions") ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "number") : [];
  } catch {
    return [];
  }
}

export const app = new AppState();
