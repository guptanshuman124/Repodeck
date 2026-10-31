<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, checkWords, errText, type CloneAccess } from "$lib/api";
  import { app } from "$lib/state.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const roots = $derived(app.roots.filter((r) => r.enabled));

  let url = $state("");
  let folder = $state(app.roots.find((r) => r.enabled)?.path ?? "");
  let name = $state("");
  let nameEdited = $state(false);
  let accountId = $state<number | null>(null);
  let results = $state<CloneAccess[] | null>(null);
  let probedUrl = $state("");
  let probing = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const sep = $derived(folder.includes("\\") ? "\\" : "/");
  const dest = $derived(folder && name ? `${folder.replace(/[\\/]+$/, "")}${sep}${name}` : "");
  const account = $derived(accountId != null ? app.accountById.get(accountId) : null);
  const byId = $derived(new Map((results ?? []).map((r) => [r.account_id, r])));
  // Accounts that can read the repo first, then by label.
  const ordered = $derived(
    [...app.accounts].sort((a, b) => {
      const ra = byId.get(a.id!)?.status === "can_read" ? 0 : 1;
      const rb = byId.get(b.id!)?.status === "can_read" ? 0 : 1;
      return ra - rb || a.label.localeCompare(b.label);
    }),
  );
  const readable = $derived((results ?? []).filter((r) => r.status === "can_read"));

  /** "git@github.com:acme/app.git" or "https://github.com/acme/app" → "app" */
  function repoName(u: string) {
    const last = u.trim().replace(/[\\/]+$/, "").split(/[\\/:]/).pop() ?? "";
    return last.replace(/\.git$/, "");
  }

  $effect(() => {
    if (!nameEdited) name = repoName(url);
  });

  async function probe() {
    const u = url.trim();
    if (!u || u === probedUrl || !app.accounts.length) return;
    probing = true;
    error = null;
    probedUrl = u;
    try {
      results = await api.probeCloneAccess(u);
      const ok = results.filter((r) => r.status === "can_read");
      if (ok.length === 1) accountId = ok[0].account_id;
      else if (accountId != null && !ok.some((r) => r.account_id === accountId)) accountId = null;
    } catch (e) {
      error = errText(e);
    } finally {
      probing = false;
    }
  }

  async function browse() {
    const p = await open({ title: "Clone into this folder", directory: true, multiple: false, defaultPath: folder || undefined });
    if (typeof p === "string") folder = p;
  }

  async function clone() {
    if (accountId == null || !dest) return;
    busy = true;
    error = null;
    try {
      await api.startClone(url.trim(), dest, accountId);
      onclose();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function badge(s: string | undefined) {
    return s === "can_read" ? "ok" : s === "offline" || s === "unknown" || !s ? "" : "error";
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" role="presentation" onclick={onclose}>
  <div class="dialog card" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <h2>Clone a repository</h2>
    <p class="muted">
      RepoDeck clones with the account you pick, and writes that account's name, email and key into the new
      repo's <code>.git/config</code>. Your global Git config is not used or changed.
    </p>

    <label>
      Repository URL
      <!-- svelte-ignore a11y_autofocus -->
      <input
        type="text"
        class="mono"
        bind:value={url}
        onblur={probe}
        onpaste={() => setTimeout(probe)}
        onkeydown={(e) => e.key === "Enter" && probe()}
        placeholder="git@github.com:acme/app.git"
        autofocus
      />
    </label>

    {#if !app.accounts.length}
      <p class="faint">Add an account first (Accounts → Add account).</p>
    {:else}
      <div class="field">
        <div class="row-flex">
          <span>Clone as</span>
          <span class="spacer"></span>
          {#if probing}<span class="faint">Checking which accounts can read it…</span>{/if}
        </div>
        <div class="accounts">
          {#each ordered as a (a.id)}
            {@const r = byId.get(a.id!)}
            <label class="acct" class:picked={accountId === a.id} title={r?.detail ?? ""}>
              <input type="radio" name="clone-account" value={a.id} bind:group={accountId} />
              <span class="who">
                <strong>{a.label}</strong>
                <span class="faint">{a.user_email} · {a.auth_method === "ssh" ? a.ssh_key_path : "HTTPS"}</span>
              </span>
              {#if r}<span class="badge {badge(r.status)}">{checkWords[r.status]}</span>{/if}
            </label>
          {/each}
        </div>
        {#if results && !readable.length}
          <p class="warn-text">
            None of your accounts can read this repo. Check the URL, and that the account's SSH key is added on
            GitHub (or that you're signed in for HTTPS). Hover an account for details.
          </p>
        {/if}
      </div>
    {/if}

    <div class="grid">
      <label>
        Into folder
        <span class="row-flex">
          <select bind:value={folder} class="grow">
            {#each roots as r (r.id)}<option value={r.path}>{r.path}</option>{/each}
            {#if folder && !roots.some((r) => r.path === folder)}<option value={folder}>{folder}</option>{/if}
          </select>
          <button type="button" onclick={browse}>Browse…</button>
        </span>
      </label>
      <label>
        Folder name
        <input type="text" class="mono" bind:value={name} oninput={() => (nameEdited = true)} placeholder="app" />
      </label>
    </div>
    {#if dest}<div class="mono faint small">{dest}</div>{/if}

    {#if account}
      <p class="summary">
        Clones as <strong>{account.user_name} &lt;{account.user_email}&gt;</strong>
        {account.auth_method === "ssh" ? `with ${account.ssh_key_path}` : "over HTTPS"}. The new repo is linked to
        {account.label} and set as active.
      </p>
    {/if}

    {#if error}<p class="err">{error}</p>{/if}

    <div class="row-flex">
      <span class="faint small">The folder must be inside one of your scan folders.</span>
      <span class="spacer"></span>
      <button onclick={onclose}>Cancel</button>
      <button class="primary" onclick={clone} disabled={busy || accountId == null || !url.trim() || !dest}>Clone</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.32);
    display: grid;
    place-items: center;
    z-index: 80;
    padding: 24px;
  }
  .dialog {
    width: min(640px, 100%);
    max-height: 100%;
    overflow: auto;
    display: grid;
    gap: 12px;
    box-shadow: var(--shadow);
  }
  p {
    margin: 0;
  }
  label,
  .field {
    display: grid;
    gap: 4px;
    font-size: 12px;
  }
  .grid {
    display: grid;
    grid-template-columns: 2fr 1fr;
    gap: 10px 14px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .accounts {
    display: grid;
    gap: 4px;
  }
  .acct {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .acct.picked {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .who {
    flex: 1;
    min-width: 0;
    display: grid;
  }
  .who .faint {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .small {
    font-size: 11px;
  }
  .summary {
    color: var(--text-2);
  }
  .warn-text {
    color: var(--warn);
    font-size: 12px;
  }
  .err {
    color: var(--error);
  }
</style>
