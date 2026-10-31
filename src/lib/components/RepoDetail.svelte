<script lang="ts">
  import { api, ago, checkWords, errText, scopeWords, type RepoDetail, type RepoRow } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import CommitGraph from "./CommitGraph.svelte";

  let { repo }: { repo: RepoRow } = $props();

  let tab = $state<"overview" | "graph" | "technical">("overview");
  let detail = $state<RepoDetail | null>(null);
  let error = $state<string | null>(null);
  let linkPick = $state<number | "">("");
  let checking = $state<number | null>(null);

  async function load() {
    try {
      detail = await api.repoDetail(repo.id);
      error = null;
    } catch (e) {
      error = errText(e);
    }
  }

  // Reload whenever the core pushes a new version of this row.
  $effect(() => {
    void repo;
    load();
  });

  const emailEntry = $derived(
    detail ? [...detail.config].reverse().find((c) => c.key.toLowerCase() === "user.email") : undefined,
  );
  const unlinked = $derived(app.accounts.filter((a) => !repo.linked_account_ids.includes(a.id!)));
  const staleDays = $derived(Number(app.settings.stale_branch_days ?? 90));
  const checkFor = (id: number) => detail?.push_checks.find((c) => c.account_id === id);
  const accountForEmail = (email: string) =>
    app.accounts.find((a) => a.user_email.toLowerCase() === email.toLowerCase());

  async function link() {
    if (linkPick === "") return;
    await app.run(() => api.linkAccount([repo.id], linkPick as number));
    linkPick = "";
  }

  async function check(accountId: number) {
    checking = accountId;
    const res = await app.run(() => api.pushCheck(repo.id, accountId));
    checking = null;
    if (res) await load();
  }

  async function unlink(accountId: number) {
    await app.run(() => api.unlinkAccount(repo.id, accountId));
  }

  function scopeText(scope: string | undefined, file: string | undefined) {
    if (!scope) return "";
    const words = scopeWords[scope] ?? scope;
    return scope === "includeIf" && file ? `${words} (${file})` : words;
  }

  function checkClass(s: string) {
    return s === "can_push" ? "ok" : s === "offline" || s === "unknown" ? "" : "error";
  }
</script>

<aside>
  <div class="head">
    <div class="titles">
      <h2 class="truncate">{repo.name}</h2>
      <div class="mono faint truncate" title={repo.path}>{repo.path}</div>
    </div>
    <button class="primary" disabled={repo.is_bare} onclick={() => app.run(() => api.openInEditor(repo.id))}>Open in VS Code</button>
    <button disabled={!repo.remote?.web_url} onclick={() => app.run(() => api.openOnGithub(repo.id, repo.head_branch ?? undefined))}>
      Open on GitHub
    </button>
    <button class="ghost close" onclick={() => (app.detailOpen = false)} aria-label="Close">✕</button>
  </div>

  <div class="tabs">
    <button class="ghost" class:on={tab === "overview"} onclick={() => (tab = "overview")}>Overview</button>
    <button class="ghost" class:on={tab === "graph"} onclick={() => (tab = "graph")}>Commit graph</button>
    <button class="ghost" class:on={tab === "technical"} onclick={() => (tab = "technical")}>Technical</button>
  </div>

  {#if tab === "graph"}
    <CommitGraph repoId={repo.id} hasGithub={!!repo.remote?.web_url} onloaded={load} />
  {:else}
    <div class="body">
      {#if error}<p class="err">{error}</p>{/if}

      {#if tab === "overview"}
        {#if repo.issues.length}
          <section class="issues">
            {#each repo.issues as i}
              <div class="issue {i.level}">
                <span>{i.text}</span>
                {#if i.fix}
                  <button class="tiny" onclick={() => app.runFix(repo.id, i.fix!)}>{i.fix.label}</button>
                {/if}
              </div>
            {/each}
          </section>
        {/if}

        <section>
          <h3>Identity</h3>
          {#if repo.is_bare}
            <p class="muted">Bare repos have no working tree, so nobody commits here directly.</p>
          {:else if repo.effective_email}
            <p class="ident">
              Commits as <strong>{repo.effective_name ?? "(no name)"}</strong>
              <span class="mono">&lt;{repo.effective_email}&gt;</span>
            </p>
            <p class="faint">Set in {scopeText(emailEntry?.scope, emailEntry?.origin_file)}</p>
          {:else}
            <p class="muted">No commit email is set anywhere for this repo.</p>
          {/if}
        </section>

        {#if !repo.is_bare}
          <section>
            <h3>Accounts</h3>
            <div class="accounts">
              {#each repo.linked_account_ids as id (id)}
                {@const a = app.accountById.get(id)}
                {@const c = checkFor(id)}
                {#if a}
                  <div class="acct" class:active={repo.active_account_id === id}>
                    <button
                      class="radio"
                      title="Make active"
                      aria-label="Make {a.label} active"
                      onclick={() => (app.switchReq = { repoId: repo.id, accountId: id })}
                    >{repo.active_account_id === id ? "●" : "○"}</button>
                    <div class="who">
                      <div><strong>{a.label}</strong> <span class="muted">{a.user_email}</span></div>
                      {#if c}
                        <div class="row-flex">
                          <span class="badge {checkClass(c.status)}">{checkWords[c.status]}</span>
                          <span class="faint small" class:stale={Date.now() / 1000 - c.checked_at > 7 * 86400}>checked {ago(c.checked_at)}</span>
                        </div>
                      {/if}
                    </div>
                    <button disabled={!repo.remote || checking != null} onclick={() => check(id)}>
                      {checking === id ? "Checking…" : c ? "Check again" : "Check push"}
                    </button>
                    <button class="ghost" disabled={repo.active_account_id === id} onclick={() => unlink(id)}>Unlink</button>
                  </div>
                {/if}
              {/each}
              <div class="acct" class:active={repo.active_account_id == null}>
                <button class="radio" aria-label="Use global identity" onclick={() => (app.switchReq = { repoId: repo.id, accountId: null })}>
                  {repo.active_account_id == null ? "●" : "○"}
                </button>
                <div class="who"><strong>Global identity</strong> <span class="muted">inherit from ~/.gitconfig</span></div>
              </div>
            </div>
            {#if unlinked.length}
              <div class="row-flex">
                <select bind:value={linkPick}>
                  <option value="">Link an account…</option>
                  {#each unlinked as a (a.id)}<option value={a.id}>{a.label} ({a.user_email})</option>{/each}
                </select>
                <button disabled={linkPick === ""} onclick={link}>Link</button>
              </div>
            {:else if !app.accounts.length}
              <p class="faint">Add accounts on the Accounts screen to switch identities here.</p>
            {/if}
          </section>

          <section>
            <h3>Working tree</h3>
            <p>
              On <strong>{repo.head_branch ?? "a detached HEAD"}</strong>,
              {repo.is_dirty == null ? "state unknown" : repo.is_dirty ? "with uncommitted changes" : "clean"}.
            </p>
          </section>
        {/if}

        <section>
          <h3>Remotes</h3>
          {#if detail?.remotes.length}
            {#each detail.remotes as r (r.name)}
              <div class="remote">
                <span class="badge">{r.name}</span>
                <span class="mono truncate">{r.url}</span>
              </div>
            {/each}
            {#if repo.last_fetch_at}
              <p class="faint small">Last fetched {ago(repo.last_fetch_at)}: {repo.last_fetch_result?.replace("_", " ")}</p>
            {/if}
          {:else}
            <p class="faint">No remotes.</p>
          {/if}
        </section>

        {#if detail?.branches.length}
          <section>
            <h3>Branches <span class="faint small">ahead/behind as of the last fetch</span></h3>
            <table>
              <tbody>
                {#each detail.branches as b (b.name)}
                  {@const stale = b.last_commit_at != null && Date.now() / 1000 - b.last_commit_at > staleDays * 86400}
                  <tr>
                    <td class="mono">
                      {b.is_head ? "● " : ""}{b.name}
                      {#if stale}<span class="badge">stale</span>{/if}
                      {#if b.gone}<span class="badge warn">upstream gone</span>{/if}
                    </td>
                    <td class="mono faint">{b.upstream ?? "no upstream"}</td>
                    <td class="num">
                      {#if b.ahead != null}
                        <span class:hot={b.ahead > 0}>↑{b.ahead}</span>
                        <span class:hot={(b.behind ?? 0) > 0}>↓{b.behind}</span>
                      {/if}
                    </td>
                    <td class="faint num">{ago(b.last_commit_at)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </section>
        {/if}

        <section>
          <h3>Commit authors</h3>
          {#if detail?.authors.length}
            <table>
              <tbody>
                {#each detail.authors as [email, n] (email)}
                  {@const acc = accountForEmail(email)}
                  {@const differs = repo.effective_email && email !== repo.effective_email.toLowerCase()}
                  <tr>
                    <td class="mono" class:warnText={differs && acc}>{email}</td>
                    <td class="faint">{acc ? acc.label : ""}</td>
                    <td class="num">{n}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
            <p class="faint small">From the commits loaded in the graph.</p>
          {:else}
            <p class="faint">Open the commit graph to analyse authors.</p>
          {/if}
        </section>
      {:else}
        <section>
          <h3>Resolved config</h3>
          <p class="faint small">
            Output of <code>git config --show-origin --show-scope</code>. Later lines win.
          </p>
          <table>
            <thead><tr><th>Key</th><th>Value</th><th>Scope</th><th>File</th></tr></thead>
            <tbody>
              {#each detail?.config ?? [] as c, i (i)}
                <tr>
                  <td class="mono">{c.key}</td>
                  <td class="mono">{c.value}</td>
                  <td>{c.scope}</td>
                  <td class="mono faint">{c.origin_file}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
        <section>
          <h3>Repo</h3>
          <table>
            <tbody>
              <tr><td>Kind</td><td>{repo.kind}{repo.parent_name ? ` (in ${repo.parent_name})` : ""}</td></tr>
              <tr><td>Path</td><td class="mono">{repo.path}</td></tr>
              {#if detail?.inspect_error}<tr><td>Error</td><td class="err">{detail.inspect_error}</td></tr>{/if}
            </tbody>
          </table>
          <div class="row-flex">
            <button onclick={() => app.run(() => api.refreshRepos([repo.id], true))}>Re-read now</button>
            <button onclick={() => app.run(() => api.revealRepo(repo.id))}>Show in folder</button>
          </div>
        </section>
      {/if}
    </div>
  {/if}
</aside>

<style>
  aside {
    border-left: 1px solid var(--border);
    background: var(--surface);
    display: grid;
    grid-template-rows: auto auto 1fr;
    min-height: 0;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 14px 6px;
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  .titles .mono {
    font-size: 11px;
  }
  .close {
    width: 28px;
    padding: 0;
    color: var(--text-2);
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    border-radius: 6px 6px 0 0;
    color: var(--text-2);
  }
  .tabs button.on {
    color: var(--text);
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .body {
    overflow: auto;
    padding: 14px;
    display: grid;
    gap: 18px;
    align-content: start;
  }
  section {
    display: grid;
    gap: 6px;
  }
  p {
    margin: 0;
  }
  .small {
    font-size: 11px;
    font-weight: normal;
  }
  .issues {
    gap: 4px;
  }
  .issue {
    padding: 6px 10px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .issue span {
    flex: 1;
  }
  .tiny {
    height: 22px;
    padding: 0 8px;
    font-size: 11px;
    flex: none;
  }
  .issue.warn {
    background: var(--warn-soft);
    color: var(--warn);
  }
  .issue.error {
    background: var(--error-soft);
    color: var(--error);
  }
  .ident {
    font-size: 14px;
  }
  .accounts {
    display: grid;
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .acct {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .acct:last-child {
    border-bottom: none;
  }
  .acct.active {
    background: var(--accent-soft);
  }
  .radio {
    border: none;
    background: none;
    width: 22px;
    padding: 0;
    color: var(--accent);
  }
  .who {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .stale {
    color: var(--warn);
  }
  .remote {
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }
  .num {
    text-align: right;
    white-space: nowrap;
  }
  .hot {
    color: var(--accent);
    font-weight: 600;
  }
  .warnText {
    color: var(--warn);
  }
  td {
    word-break: break-word;
  }
  .err {
    color: var(--error);
  }
</style>
