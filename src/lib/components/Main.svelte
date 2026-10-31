<script lang="ts">
  import { api } from "$lib/api";
  import { app, type GroupBy } from "$lib/state.svelte";
  import RepoList from "./RepoList.svelte";
  import RepoDetail from "./RepoDetail.svelte";
  import Accounts from "./Accounts.svelte";
  import Settings from "./Settings.svelte";
  import JobBar from "./JobBar.svelte";
  import SwitchDialog from "./SwitchDialog.svelte";
  import BulkSwitchDialog from "./BulkSwitchDialog.svelte";
  import CloneDialog from "./CloneDialog.svelte";

  let search: HTMLInputElement | undefined = $state();
  let popover = $state<"filter" | "more" | null>(null);
  let bulkPick = $state<string>("");

  const mod = navigator.platform.startsWith("Mac") ? "⌘" : "Ctrl";
  const fetchRunning = $derived(app.jobs.some((j) => j.kind === "fetch" && !j.finished));
  const scanRunning = $derived(app.jobs.some((j) => j.kind === "scan" && !j.finished));
  const picked = $derived(app.selectedIds.size);
  const rootName = (id: number) => {
    const r = app.roots.find((x) => x.id === id);
    return r ? r.path.split(/[\\/]/).pop() || r.path : "folder";
  };
  const accountFilterName = $derived(
    app.accountFilter === "none"
      ? "Global identity"
      : typeof app.accountFilter === "number"
        ? (app.accountById.get(app.accountFilter)?.label ?? "account")
        : "",
  );
  const filterCount = $derived(
    Number(app.rootFilter !== "all") + Number(app.accountFilter !== "all") + Number(app.attentionOnly),
  );

  function fetchIds(ids: number[]) {
    const withRemote = ids.filter((id) => app.repoById.get(id)?.remote);
    if (!withRemote.length) return app.notify("None of these repos has a remote.");
    app.run(() => api.startFetch(withRemote));
  }

  async function reReadAll() {
    popover = null;
    const n = await app.run(() => api.refreshRepos(undefined, true));
    if (n !== undefined) app.notify(`Re-read ${n} repo${n === 1 ? "" : "s"}`);
  }

  function rescan() {
    popover = null;
    app.run(() => api.startScan());
  }

  function bulkSwitch() {
    if (bulkPick === "") return;
    app.bulkReq = { repoIds: [...app.selectedIds], accountId: bulkPick === "global" ? null : Number(bulkPick) };
    bulkPick = "";
  }

  function isTyping(e: KeyboardEvent) {
    const t = e.target as HTMLElement | null;
    return !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
  }

  function onKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      app.view = "repos";
      search?.focus();
      search?.select();
      return;
    }
    if (app.switchReq || app.bulkReq || app.cloneOpen) return;
    if (e.key === "Escape") {
      popover = null;
      if (isTyping(e)) (e.target as HTMLElement).blur();
      else if (app.selectedIds.size) app.selectedIds.clear();
      else if (app.detailOpen) app.detailOpen = false;
      return;
    }
    if (app.view !== "repos") return;
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a" && !isTyping(e)) {
      e.preventDefault();
      for (const v of app.visible) app.selectedIds.add(v.repo.id);
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const list = app.visible;
    const idx = list.findIndex((v) => v.repo.id === app.selectedId);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (isTyping(e) && e.target !== search) return;
      e.preventDefault();
      const next = e.key === "ArrowDown" ? Math.min(list.length - 1, idx + 1) : Math.max(0, idx - 1);
      if (list[next]) app.selectedId = list[next].repo.id;
      return;
    }
    if (e.key === "Enter" && app.selectedId != null) {
      if (isTyping(e) && e.target !== search) return;
      e.preventDefault();
      app.detailOpen = true;
      return;
    }
    if (isTyping(e) || app.selectedId == null) return;
    const r = app.selected;
    if (!r) return;
    if (e.key === "o" || e.key === "O") {
      e.preventDefault();
      app.run(() => api.openInEditor(r.id));
    } else if (e.key === "g" || e.key === "G") {
      e.preventDefault();
      if (r.remote?.web_url) app.run(() => api.openOnGithub(r.id));
      else app.notify("This repo has no GitHub remote.");
    }
  }
</script>

<svelte:window onkeydown={onKey} onclick={() => (popover = null)} />

<div class="shell">
  <header>
    <nav>
      <strong class="brand">RepoDeck</strong>
      <button class="ghost" class:on={app.view === "repos"} onclick={() => (app.view = "repos")}>Repos</button>
      <button class="ghost" class:on={app.view === "accounts"} onclick={() => (app.view = "accounts")}>
        Accounts{#if app.unhealthyAccounts.length}<span class="alert" title="{app.unhealthyAccounts.length} account{app.unhealthyAccounts.length === 1 ? '' : 's'} can't sign in"></span>{/if}
      </button>
      <button class="ghost" class:on={app.view === "settings"} onclick={() => (app.view = "settings")}>Settings</button>
    </nav>

    {#if app.view === "repos"}
      {#if picked >= 2}
        <div class="tools selection">
          <strong>{picked} selected</strong>
          <select bind:value={bulkPick} onchange={bulkSwitch} title="Switch every selected repo to one account">
            <option value="">Use account…</option>
            {#each app.accounts as a (a.id)}<option value={String(a.id)}>{a.label}</option>{/each}
            <option value="global">Global identity</option>
          </select>
          <button onclick={() => fetchIds([...app.selectedIds])} disabled={fetchRunning}>Fetch</button>
          <span class="spacer"></span>
          <span class="faint small">{mod}-click to add or remove · Shift-click for a range · Esc to clear</span>
          <button class="ghost" onclick={() => app.selectedIds.clear()}>Clear</button>
        </div>
      {:else}
        <div class="tools">
          <input
            bind:this={search}
            type="search"
            class="search"
            placeholder="Search repos, paths, emails, owners"
            bind:value={app.query}
          />
          <kbd class="hint">{mod} K</kbd>

          <div class="pop-wrap">
            <button
              class:active={filterCount > 0}
              onclick={(e) => {
                e.stopPropagation();
                popover = popover === "filter" ? null : "filter";
              }}
            >
              Filter{#if filterCount} · {filterCount}{/if} ▾
            </button>
            {#if popover === "filter"}
              <div class="pop card" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
                <label>
                  Folder
                  <select bind:value={app.rootFilter}>
                    <option value="all">All folders</option>
                    {#each app.roots as r (r.id)}<option value={r.id}>{rootName(r.id)}</option>{/each}
                  </select>
                </label>
                <label>
                  Account
                  <select bind:value={app.accountFilter}>
                    <option value="all">All accounts</option>
                    <option value="none">Using global identity</option>
                    {#each app.accounts as a (a.id)}<option value={a.id}>{a.label}</option>{/each}
                  </select>
                </label>
                <label class="check">
                  <input type="checkbox" bind:checked={app.attentionOnly} />
                  Only repos that need attention
                </label>
              </div>
            {/if}
          </div>

          <label class="group-by">
            Group
            <select value={app.groupBy} onchange={(e) => app.setGroupBy(e.currentTarget.value as GroupBy)}>
              <option value="none">None</option>
              <option value="owner">By owner</option>
              <option value="account">By account</option>
              <option value="folder">By folder</option>
            </select>
          </label>

          <span class="spacer"></span>
          <button class="primary" onclick={() => (app.cloneOpen = true)} title="Clone a repo as one of your accounts">Clone…</button>
          <button onclick={() => fetchIds(app.visible.map((v) => v.repo.id))} disabled={fetchRunning} title="git fetch --all in every repo shown">
            Fetch all
          </button>
          <div class="pop-wrap">
            <button
              class="ghost"
              aria-label="More"
              onclick={(e) => {
                e.stopPropagation();
                popover = popover === "more" ? null : "more";
              }}>⋯</button
            >
            {#if popover === "more"}
              <div class="pop card menu right" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
                <button class="item" onclick={rescan} disabled={scanRunning}>
                  Look for new repos <span class="faint">rescan your folders</span>
                </button>
                <button class="item" onclick={reReadAll}>
                  Re-read every repo <span class="faint">ignore the cache</span>
                </button>
              </div>
            {/if}
          </div>
        </div>
      {/if}

      <div class="summary">
        <span>{app.repos.length} repo{app.repos.length === 1 ? "" : "s"}</span>
        {#if app.attentionCount}
          · <button class="link warnText" onclick={() => (app.attentionOnly = true)}>{app.attentionCount} need attention</button>
        {:else if app.repos.length}
          · <span class="okText">all identities match</span>
        {/if}
        {#if app.unhealthyAccounts.length}
          · <button class="link errText" onclick={() => (app.view = "accounts")}>
            {app.unhealthyAccounts.length} account{app.unhealthyAccounts.length === 1 ? "" : "s"} can't sign in
          </button>
        {/if}
        {#if app.rootFilter !== "all"}
          <span class="chip">{rootName(app.rootFilter)} <button aria-label="Remove filter" onclick={() => (app.rootFilter = "all")}>✕</button></span>
        {/if}
        {#if app.accountFilter !== "all"}
          <span class="chip">{accountFilterName} <button aria-label="Remove filter" onclick={() => (app.accountFilter = "all")}>✕</button></span>
        {/if}
        {#if app.attentionOnly}
          <span class="chip">Needs attention <button aria-label="Remove filter" onclick={() => (app.attentionOnly = false)}>✕</button></span>
        {/if}
        {#if app.query.trim()}
          <span class="faint">· {app.visible.length} shown</span>
        {/if}
      </div>

      {#if !app.accounts.length && app.repos.length}
        <div class="suggest">
          <span>Every repo uses your global Git identity. Add an account to switch identities per repo.</span>
          <span class="spacer"></span>
          <button class="primary" onclick={() => app.editAccount("new")}>Add your first account</button>
        </div>
      {/if}
      {#each app.suggestions as s (s.accountId)}
        {@const a = app.accountById.get(s.accountId)}
        {#if a}
          <div class="suggest">
            <span>
              <strong>{s.repoIds.length} repo{s.repoIds.length === 1 ? "" : "s"}</strong> look like
              <strong>{a.label}</strong> repos but don't use it yet.
            </span>
            <span class="spacer"></span>
            <button class="primary" onclick={() => (app.bulkReq = { repoIds: s.repoIds, accountId: s.accountId })}>
              Review and use {a.label}
            </button>
            <button class="ghost" onclick={() => app.dismissSuggestion(s.accountId)} aria-label="Dismiss">Not now</button>
          </div>
        {/if}
      {/each}
    {/if}
  </header>

  <main>
    {#if app.view === "repos"}
      <div class="split" class:open={app.detailOpen && app.selected}>
        <RepoList />
        {#if app.detailOpen && app.selected}
          {#key app.selected.id}
            <RepoDetail repo={app.selected} />
          {/key}
        {/if}
      </div>
    {:else if app.view === "accounts"}
      <Accounts />
    {:else}
      <Settings />
    {/if}
  </main>

  <JobBar />
</div>

{#if app.switchReq}
  <SwitchDialog req={app.switchReq} onclose={() => (app.switchReq = null)} />
{/if}

{#if app.bulkReq}
  <BulkSwitchDialog req={app.bulkReq} onclose={() => (app.bulkReq = null)} />
{/if}

{#if app.cloneOpen}
  <CloneDialog onclose={() => (app.cloneOpen = false)} />
{/if}

<style>
  .alert {
    display: inline-block;
    width: 7px;
    height: 7px;
    margin-left: 5px;
    border-radius: 50%;
    background: var(--error);
    vertical-align: 2px;
  }
  .shell {
    height: 100vh;
    display: grid;
    grid-template-rows: auto 1fr auto;
  }
  header {
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  nav {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 14px 0;
  }
  .brand {
    margin-right: 12px;
    font-size: 14px;
  }
  nav button {
    border-radius: 6px 6px 0 0;
    color: var(--text-2);
  }
  nav button.on {
    color: var(--text);
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    flex-wrap: wrap;
  }
  .search {
    width: 280px;
  }
  .hint {
    margin-left: -4px;
  }
  .tools.selection {
    background: var(--accent-soft);
  }
  .small {
    font-size: 11px;
  }
  .pop-wrap {
    position: relative;
  }
  .pop {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 60;
    min-width: 240px;
    display: grid;
    gap: 10px;
    padding: 12px;
    box-shadow: var(--shadow);
  }
  .pop.right {
    left: auto;
    right: 0;
  }
  .pop label {
    display: grid;
    gap: 4px;
    font-size: 12px;
  }
  .pop label.check {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .menu {
    padding: 4px;
    gap: 0;
  }
  .item {
    display: grid;
    justify-items: start;
    text-align: left;
    height: auto;
    padding: 6px 10px;
    border: none;
    background: transparent;
    width: 100%;
  }
  .item:hover:not(:disabled) {
    background: var(--row-hover);
  }
  .item .faint {
    font-size: 11px;
  }
  button.active {
    border-color: var(--accent);
    color: var(--accent);
  }
  .group-by {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
  }
  .summary {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    padding: 0 14px 8px;
    font-size: 12px;
    color: var(--text-2);
  }
  .link {
    padding: 0;
    height: auto;
    border: none;
    background: none;
    font-size: 12px;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .warnText {
    color: var(--warn);
  }
  .errText {
    color: var(--error);
  }
  .okText {
    color: var(--ok);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 4px 1px 8px;
    border-radius: 10px;
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .chip button {
    height: 16px;
    padding: 0 4px;
    border: none;
    background: none;
    font-size: 10px;
  }
  .suggest {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 14px 10px;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--accent-soft);
    font-size: 13px;
  }
  main {
    min-height: 0;
    overflow: hidden;
  }
  .split {
    height: 100%;
    display: grid;
    grid-template-columns: 1fr;
  }
  .split.open {
    grid-template-columns: minmax(360px, 1fr) minmax(420px, 1.1fr);
  }
</style>
