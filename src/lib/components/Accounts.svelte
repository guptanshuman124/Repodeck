<script lang="ts">
  import { onMount } from "svelte";
  import { api, ago, checkWords, type Account, type MatrixCell } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import AccountForm from "./AccountForm.svelte";
  import AccountWizard from "./AccountWizard.svelte";

  let editing = $state<Account | "new" | null>(null);
  let matrix = $state<MatrixCell[]>([]);
  let filter = $state("");
  let confirmDelete = $state<number | null>(null);
  let testingId = $state<number | null>(null);

  // Other screens ask for an account editor through app.editAccount().
  $effect(() => {
    const want = app.editAccountId;
    if (want == null) return;
    editing = want === "new" ? "new" : (app.accountById.get(want) ?? null);
    app.editAccountId = null;
    document.querySelector(".page")?.scrollTo({ top: 0 });
  });

  async function testAccount(a: Account) {
    if (!a.ssh_key_path) return;
    testingId = a.id;
    await app.run(() => api.testSshKey(a.ssh_key_path!, a.host, a.expected_username, a.id));
    testingId = null;
    await app.reloadAccounts();
  }

  async function repairKey(a: Account) {
    const n = await app.run(() => api.repairAccountKey(a.id!));
    if (n === undefined) return;
    await app.reloadAccounts();
    app.notify(`${a.label} now uses its private key${n ? `; updated ${n} repo${n === 1 ? "" : "s"}` : ""}`);
  }

  function healthClass(level: string | undefined) {
    return level === "ok" ? "ok" : level === "error" ? "error" : level === "warn" ? "warn" : "";
  }

  const accessJob = $derived(app.jobs.find((j) => j.kind === "access" && !j.finished));
  const githubRepos = $derived(app.repos.filter((r) => r.remote?.is_github));
  const cells = $derived(new Map(matrix.map((c) => [`${c.repo_id}:${c.account_id}`, c])));
  const matrixRepos = $derived(
    githubRepos.filter((r) => {
      const q = filter.trim().toLowerCase();
      return !q || r.name.toLowerCase().includes(q) || (r.remote?.owner ?? "").toLowerCase().includes(q);
    }),
  );

  /** Accounts that can push to a repo but are not linked yet. */
  const suggestions = $derived.by(() => {
    const out: { repoId: number; accountId: number; makeActive: boolean }[] = [];
    for (const r of githubRepos) {
      const can = app.accounts.filter((a) => cells.get(`${r.id}:${a.id}`)?.status === "can_push");
      for (const a of can) {
        if (!r.linked_account_ids.includes(a.id!)) {
          out.push({ repoId: r.id, accountId: a.id!, makeActive: can.length === 1 && r.active_account_id == null });
        }
      }
    }
    return out;
  });

  async function loadMatrix() {
    matrix = (await app.run(() => api.accessMatrix())) ?? [];
  }

  onMount(loadMatrix);

  // Refresh the matrix as the access scan reports progress.
  $effect(() => {
    const j = app.jobs.find((x) => x.kind === "access");
    if (j && (j.done % 10 === 0 || j.finished)) loadMatrix();
  });

  async function linkAll() {
    const byAccount = new Map<number, number[]>();
    for (const s of suggestions) byAccount.set(s.accountId, [...(byAccount.get(s.accountId) ?? []), s.repoId]);
    for (const [acc, repos] of byAccount) await app.run(() => api.linkAccount(repos, acc));
    app.notify(`Linked ${suggestions.length} account${suggestions.length === 1 ? "" : "s"}`);
  }

  async function remove(id: number) {
    await app.run(() => api.deleteAccount(id));
    confirmDelete = null;
    await app.reloadAccounts();
    await loadMatrix();
  }

  function cellClass(s: string | undefined) {
    if (!s) return "";
    return s === "can_push" ? "ok" : s === "offline" || s === "unknown" ? "" : s === "read_only" ? "warn" : "error";
  }
</script>

<div class="page">
  <section class="block">
    <div class="row-flex">
      <h2>Accounts</h2>
      <span class="spacer"></span>
      {#if editing === null}<button class="primary" onclick={() => (editing = "new")}>Add account</button>{/if}
    </div>
    <p class="muted">
      Each account is a saved identity. Linking one to a repo and making it active writes its name,
      email and SSH key into that repo's <code>.git/config</code> only. RepoDeck never edits
      <code>~/.gitconfig</code> or <code>~/.ssh/config</code>.
    </p>

    {#if editing !== null}
      <div class="card">
        {#key editing}
          {#if editing === "new"}
            <AccountWizard
              onsaved={async () => {
                editing = null;
                await app.reloadAccounts();
              }}
              oncancel={() => (editing = null)}
            />
          {:else}
            <h3 class="edit-title">Edit {editing.label}</h3>
            <AccountForm
              account={editing}
              onsaved={async () => {
                editing = null;
                await app.reloadAccounts();
              }}
              oncancel={() => (editing = null)}
            />
          {/if}
        {/key}
      </div>
    {/if}

    {#if app.accounts.length}
      <div class="list">
        {#each app.accounts as a (a.id)}
          {@const n = app.repos.filter((r) => r.active_account_id === a.id).length}
          {@const h = a.health}
          <div class="acct card" class:bad={h?.level === "error"}>
            <div class="who">
              <div class="row-flex">
                <strong>{a.label}</strong>
                {#if h}
                  <span class="badge {healthClass(h.level)}" title={h.checked_at ? `Tested ${ago(h.checked_at)}` : ""}>
                    {h.level === "ok" ? "OK" : h.kind === "missing" ? "Key missing" : h.kind === "public_key" ? "Public key" : h.kind === "no_key" ? "No key" : h.kind === "untested" ? "Not tested" : (checkWords[h.kind as keyof typeof checkWords] ?? h.kind)}
                  </span>
                  <span class="faint small">{h.level === "ok" && h.checked_at ? `${h.text} · tested ${ago(h.checked_at)}` : h.level === "unknown" ? "" : h.text}</span>
                {/if}
              </div>
              <span class="muted">{a.user_name} &lt;{a.user_email}&gt;</span>
              <span class="faint small">
                {a.auth_method === "ssh" ? `SSH · ${a.ssh_key_path}` : "HTTPS · credential helper"}
                {a.expected_username ? ` · @${a.expected_username}` : ""} · {a.host}
                {a.owners ? ` · expected for ${a.owners}` : ""}
              </span>
            </div>
            <span class="badge">{n} active repo{n === 1 ? "" : "s"}</span>
            {#if h?.kind === "public_key" && h.fix_path}
              <button class="primary" onclick={() => repairKey(a)} title="Use {h.fix_path}">Fix key</button>
            {:else if a.auth_method === "ssh" && h?.kind !== "missing" && h?.kind !== "no_key"}
              <button onclick={() => testAccount(a)} disabled={testingId != null}>{testingId === a.id ? "Testing…" : "Test"}</button>
            {/if}
            <button class:primary={h?.level === "error" && h.kind !== "public_key"} onclick={() => (editing = a)}>Edit</button>
            {#if confirmDelete === a.id}
              <button class="danger" onclick={() => remove(a.id!)}>Delete profile</button>
              <button class="ghost" onclick={() => (confirmDelete = null)}>Keep</button>
            {:else}
              <button class="ghost danger" onclick={() => (confirmDelete = a.id)}>Delete</button>
            {/if}
          </div>
        {/each}
      </div>
    {:else if editing === null}
      <p class="faint">No accounts yet. Repos use your global Git identity until you add one.</p>
    {/if}
  </section>

  {#if app.accounts.length}
    <section class="block">
      <div class="row-flex">
        <h2>Who can push where</h2>
        <span class="spacer"></span>
        {#if accessJob}
          <span class="faint">{accessJob.label}</span>
          <button onclick={() => api.cancelJob(accessJob.id)}>Cancel</button>
        {:else}
          <button onclick={() => app.run(() => api.startAccessScan(true))} title="Skips pairs checked in the last 7 days">
            Scan changed
          </button>
          <button onclick={() => app.run(() => api.startAccessScan(false))}>Scan all</button>
        {/if}
      </div>
      <p class="muted">
        Checks every account against every GitHub repo with a dry-run push, which never writes anything.
        {githubRepos.length} repos × {app.accounts.length} accounts = {githubRepos.length * app.accounts.length} checks. Needs the network.
      </p>

      {#if suggestions.length}
        <div class="suggest card">
          <strong>{suggestions.length} suggestion{suggestions.length === 1 ? "" : "s"}</strong>
          <span class="muted">Accounts that can push to repos they aren't linked to yet.</span>
          <span class="spacer"></span>
          <button class="primary" onclick={linkAll}>Link all</button>
        </div>
      {/if}

      <input type="search" placeholder="Filter repos" bind:value={filter} class="filter" />
      <div class="matrix">
        <table>
          <thead>
            <tr>
              <th>Repo</th>
              {#each app.accounts as a (a.id)}<th>{a.label}</th>{/each}
            </tr>
          </thead>
          <tbody>
            {#each matrixRepos as r (r.id)}
              <tr>
                <td>
                  <div class="truncate"><strong>{r.name}</strong></div>
                  <div class="faint small truncate">{r.remote?.owner}/{r.remote?.repo}</div>
                </td>
                {#each app.accounts as a (a.id)}
                  {@const c = cells.get(`${r.id}:${a.id}`)}
                  {@const linked = r.linked_account_ids.includes(a.id!)}
                  <td title={c ? `${c.detail ?? ""}\nChecked ${ago(c.checked_at)}` : "Not checked"}>
                    <div class="cell">
                      {#if c}
                        <span class="badge {cellClass(c.status)}">{checkWords[c.status]}</span>
                      {:else}
                        <span class="faint">—</span>
                      {/if}
                      {#if r.active_account_id === a.id}
                        <span class="badge accent">active</span>
                      {:else if c?.status === "can_push" && !linked}
                        <button class="ghost tiny" onclick={() => app.run(() => api.linkAccount([r.id], a.id!))}>Link</button>
                      {:else if c?.status === "can_push" && linked}
                        <button class="ghost tiny" onclick={() => (app.switchReq = { repoId: r.id, accountId: a.id })}>Make active</button>
                      {/if}
                    </div>
                  </td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
        {#if !githubRepos.length}<p class="faint">No repos with a GitHub remote yet.</p>{/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .page {
    height: 100%;
    overflow: auto;
    padding: 18px 22px 40px;
    display: grid;
    gap: 28px;
    align-content: start;
  }
  .block {
    display: grid;
    gap: 10px;
    max-width: 1100px;
  }
  p {
    margin: 0;
  }
  .small {
    font-size: 11px;
  }
  .list {
    display: grid;
    gap: 8px;
  }
  .acct.bad {
    border-color: var(--error);
  }
  .acct {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
  }
  .who {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .suggest {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: var(--accent-soft);
    border-color: transparent;
  }
  .filter {
    width: 260px;
  }
  .matrix {
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
  }
  .matrix td:first-child {
    max-width: 280px;
  }
  .cell {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .tiny {
    height: 20px;
    padding: 0 6px;
    font-size: 11px;
    color: var(--accent);
  }
  .edit-title {
    margin: 0 0 10px;
    font-size: 14px;
  }
</style>
