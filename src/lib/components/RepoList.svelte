<script lang="ts">
  import { api, ago, type RepoRow } from "$lib/api";
  import { app } from "$lib/state.svelte";

  const ROW = 44;
  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let height = $state(600);

  const rows = $derived(app.listRows);
  const start = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 8));
  const end = $derived(Math.min(rows.length, Math.ceil((scrollTop + height) / ROW) + 8));

  let menu = $state<{ repo: RepoRow; kind: "account" | "more"; x: number; y: number } | null>(null);

  // Keep the keyboard selection in view.
  $effect(() => {
    const id = app.selectedId;
    if (!viewport || id == null) return;
    const i = rows.findIndex((v) => v.kind === "repo" && v.repo.id === id);
    if (i < 0) return;
    const top = i * ROW;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + ROW > viewport.scrollTop + viewport.clientHeight)
      viewport.scrollTop = top + ROW - viewport.clientHeight;
  });

  function openMenu(e: MouseEvent, repo: RepoRow, kind: "account" | "more") {
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { repo, kind, x: Math.min(r.left, window.innerWidth - 260), y: r.bottom + 4 };
    app.selectedId = repo.id;
  }

  function select(r: RepoRow) {
    app.selectedId = r.id;
    app.detailOpen = true;
  }

  /** Plain click opens a repo; ⌘/Ctrl-click toggles it into the selection; Shift-click picks a range. */
  function click(e: MouseEvent, r: RepoRow) {
    const sel = app.selectedIds;
    if (e.metaKey || e.ctrlKey) {
      if (!sel.size && app.selectedId != null && app.selectedId !== r.id) sel.add(app.selectedId);
      if (sel.has(r.id)) sel.delete(r.id);
      else sel.add(r.id);
      app.selectedId = r.id;
      return;
    }
    if (e.shiftKey && app.selectedId != null) {
      const ids = app.visible.map((v) => v.repo.id);
      const [a, b] = [ids.indexOf(app.selectedId), ids.indexOf(r.id)].sort((x, y) => x - y);
      if (a >= 0) for (const id of ids.slice(a, b + 1)) sel.add(id);
      return;
    }
    sel.clear();
    select(r);
  }

  function identityText(r: RepoRow): string {
    if (r.is_bare) return "Bare repo";
    if (!r.inspected) return "Reading…";
    const a = r.active_account_id != null ? app.accountById.get(r.active_account_id) : null;
    if (a) return a.label;
    if (r.effective_email) return r.effective_email;
    return "No identity";
  }

  function kindLabel(r: RepoRow): string | null {
    switch (r.kind) {
      case "submodule":
        return "submodule";
      case "worktree":
        return "worktree";
      case "bare":
        return "bare";
      default:
        return null;
    }
  }

  function act(f: () => Promise<unknown>) {
    app.run(f); // calls f synchronously, before the menu closes
    menu = null;
  }
</script>

<svelte:window onclick={() => (menu = null)} onresize={() => viewport && (height = viewport.clientHeight)} />

<div
  class="viewport"
  bind:this={viewport}
  bind:clientHeight={height}
  onscroll={() => (scrollTop = viewport!.scrollTop)}
  role="listbox"
  aria-label="Repositories"
  tabindex="-1"
>
  {#if rows.length === 0}
    <div class="empty">
      {#if app.repos.length === 0}
        <p>No repos yet. Add a folder in Settings, then Rescan.</p>
      {:else if app.attentionOnly}
        <p>Nothing needs attention. Every identity matches.</p>
      {:else}
        <p>No repos match this search.</p>
      {/if}
    </div>
  {/if}
  <div style="height: {rows.length * ROW}px; position: relative;">
    {#each rows.slice(start, end) as row, i (row.kind === "repo" ? row.repo.id : `h:${row.key}`)}
      {#if row.kind === "header"}
        <button
          class="group ghost"
          style="top: {(start + i) * ROW}px"
          onclick={() => app.toggleGroup(row.key)}
          aria-expanded={!row.collapsed}
        >
          <span class="chev" class:closed={row.collapsed}>▾</span>
          <strong class="truncate">{row.key}</strong>
          <span class="faint">{row.count}</span>
          {#if row.problems}<span class="badge warn">{row.problems} need attention</span>{/if}
        </button>
      {:else}
      {@const r = row.repo}
      {@const depth = row.depth}
      {@const a = r.active_account_id != null ? app.accountById.get(r.active_account_id) : null}
      <div
        class="row"
        class:selected={app.selectedId === r.id}
        class:picked={app.selectedIds.has(r.id)}
        style="top: {(start + i) * ROW}px; padding-left: {14 + depth * 22}px"
        onclick={(e) => click(e, r)}
        onkeydown={(e) => e.key === "Enter" && select(r)}
        role="option"
        aria-selected={app.selectedId === r.id}
        tabindex="-1"
      >
        <span class="dot {r.status}" title={r.issues.map((x) => x.text).join("\n")}></span>
        <div class="name">
          <div class="row-flex line">
            <span class="truncate title">{r.name}</span>
            {#if kindLabel(r)}<span class="badge">{kindLabel(r)}</span>{/if}
            {#if r.kind === "nested" && r.parent_name}<span class="badge">nested in {r.parent_name}</span>{/if}
          </div>
          <div class="sub truncate">
            {#if r.issues.length}
              {@const fix = r.issues.find((i) => i.fix)?.fix}
              <span class={r.status}>{r.issues[0].text}</span>
              {#if fix}
                <button
                  class="fix"
                  onclick={(e) => {
                    e.stopPropagation();
                    app.selectedId = r.id;
                    app.runFix(r.id, fix);
                  }}>{fix.label}</button
                >
              {/if}
            {:else}
              <span class="faint">{r.head_branch ?? (r.is_bare ? "" : "detached")}{r.is_dirty ? " · uncommitted changes" : ""}</span>
            {/if}
          </div>
        </div>
        <button
          class="ident ghost truncate"
          class:muted={!a}
          disabled={r.is_bare}
          onclick={(e) => openMenu(e, r, "account")}
          title={r.effective_email ? `Commits as ${r.effective_name ?? ""} <${r.effective_email}>` : ""}
        >
          {identityText(r)}
          {#if r.linked_account_ids.length > 1}<span class="badge">+{r.linked_account_ids.length - 1}</span>{/if}
          <span class="caret">▾</span>
        </button>
        <span class="when faint">{ago(r.last_commit_at)}</span>
        <button class="ghost more" onclick={(e) => openMenu(e, r, "more")} aria-label="More actions">⋯</button>
      </div>
      {/if}
    {/each}
  </div>
</div>

{#if menu}
  {@const r = menu.repo}
  <div class="menu" style="left: {menu.x}px; top: {menu.y}px" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    {#if menu.kind === "account"}
      <div class="menu-head">Commit and push as</div>
      {#each r.linked_account_ids as id (id)}
        {@const acc = app.accountById.get(id)}
        {#if acc}
          <button class="item" onclick={() => { app.switchReq = { repoId: r.id, accountId: id }; menu = null; }}>
            <span class="check">{r.active_account_id === id ? "●" : ""}</span>
            {acc.label} <span class="faint">{acc.user_email}</span>
          </button>
        {/if}
      {/each}
      <button class="item" onclick={() => { app.switchReq = { repoId: r.id, accountId: null }; menu = null; }}>
        <span class="check">{r.active_account_id == null ? "●" : ""}</span>
        Use global identity
      </button>
      <div class="sep"></div>
      <button class="item" onclick={() => { select(r); menu = null; }}>Link accounts…</button>
    {:else}
      <button class="item" disabled={r.is_bare} onclick={() => act(() => api.openInEditor(r.id))}>Open in VS Code <kbd>O</kbd></button>
      <button class="item" disabled={!r.remote?.web_url} onclick={() => act(() => api.openOnGithub(r.id))}>Open on GitHub <kbd>G</kbd></button>
      <button class="item" onclick={() => act(() => api.revealRepo(r.id))}>Show in folder</button>
      <div class="sep"></div>
      <button class="item" onclick={() => act(() => api.refreshRepos([r.id], true))}>Refresh</button>
      <button class="item" disabled={!r.remote} onclick={() => act(() => api.startFetch([r.id]))}>Fetch</button>
      {#if r.active_account_id != null}
        <button class="item" disabled={!r.remote} onclick={() => act(async () => {
          const res = await api.pushCheck(r.id, r.active_account_id!);
          app.notify(`${app.accountById.get(r.active_account_id!)?.label}: ${res.detail}`);
        })}>Check push access</button>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .fix {
    height: 18px;
    padding: 0 7px;
    margin-left: 6px;
    font-size: 11px;
    border-radius: 9px;
    border: 1px solid var(--accent);
    color: var(--accent);
    background: transparent;
    vertical-align: 1px;
  }
  .fix:hover {
    background: var(--accent-soft);
  }
  .viewport {
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    position: relative;
    outline: none;
  }
  .empty {
    padding: 48px;
    text-align: center;
    color: var(--text-2);
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 44px;
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) minmax(90px, 220px) 64px 28px;
    align-items: center;
    gap: 12px;
    padding-right: 10px;
    border-bottom: 1px solid var(--border);
    cursor: default;
  }
  .row:hover {
    background: var(--row-hover);
  }
  .row.selected {
    background: var(--row-selected);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: transparent;
    border: 1.5px solid var(--border-strong);
  }
  .dot.warn {
    background: var(--warn);
    border-color: var(--warn);
  }
  .dot.error {
    background: var(--error);
    border-color: var(--error);
  }
  .dot.pending {
    border-style: dashed;
  }
  .name {
    min-width: 0;
  }
  .line {
    gap: 6px;
  }
  .title {
    font-weight: 500;
  }
  .sub {
    font-size: 12px;
  }
  .sub .warn {
    color: var(--warn);
  }
  .sub .error {
    color: var(--error);
  }
  .ident {
    justify-self: stretch;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
  }
  .ident.muted {
    color: var(--text-2);
  }
  .caret {
    margin-left: auto;
    color: var(--text-3);
    font-size: 10px;
  }
  .when {
    font-size: 12px;
    text-align: right;
  }
  .more {
    padding: 0;
    width: 28px;
    color: var(--text-2);
  }
  .menu {
    position: fixed;
    z-index: 50;
    min-width: 240px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: grid;
  }
  .menu-head {
    font-size: 11px;
    color: var(--text-3);
    padding: 4px 8px;
  }
  .item {
    border: none;
    background: none;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 6px;
    border-radius: 5px;
  }
  .item kbd {
    margin-left: auto;
  }
  .check {
    width: 12px;
    color: var(--accent);
    font-size: 9px;
  }
  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 0;
  }
  .row.picked {
    background: var(--accent-soft);
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .group {
    position: absolute;
    left: 0;
    right: 0;
    height: 44px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 14px;
    border-radius: 0;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 12px;
    text-align: left;
  }
  .chev {
    display: inline-block;
    width: 12px;
    transition: transform 0.12s;
  }
  .chev.closed {
    transform: rotate(-90deg);
  }
</style>
