<script lang="ts">
  import { onMount } from "svelte";
  import { api, errText, type BulkItem } from "$lib/api";
  import { app } from "$lib/state.svelte";

  let { req, onclose }: { req: { repoIds: number[]; accountId: number | null }; onclose: () => void } = $props();

  // The dialog starts on the requested account; the picker can change it afterwards.
  const initial = () => req.accountId;
  let accountId = $state<number | null>(initial());
  let items = $state<BulkItem[] | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let showDetails = $state(false);

  const account = $derived(accountId != null ? app.accountById.get(accountId) : null);
  const changing = $derived((items ?? []).filter((i) => i.changes.length));
  const matching = $derived((items ?? []).length - changing.length);

  async function load() {
    items = null;
    error = null;
    try {
      items = await api.previewBulkSwitch(req.repoIds, accountId);
    } catch (e) {
      error = errText(e);
    }
  }

  onMount(load);

  async function apply() {
    if (!items) return;
    busy = true;
    try {
      const res = await api.applyBulkSwitch(accountId, items);
      app.selectedIds.clear();
      const who = account ? account.label : "the global identity";
      if (res.skipped.length) {
        app.notify(
          `${res.applied} repo${res.applied === 1 ? "" : "s"} now use ${who}. Skipped ${res.skipped.map((s) => s[0]).join(", ")}: their config changed since the preview.`,
          true,
        );
      } else {
        app.notify(`${res.applied} repo${res.applied === 1 ? "" : "s"} now use ${who}`);
      }
      onclose();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" role="presentation" onclick={onclose}>
  <div class="dialog card" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <h2>Switch {req.repoIds.length} repos</h2>

    <label class="pick">
      Use
      <select
        value={accountId ?? ""}
        onchange={(e) => {
          const v = e.currentTarget.value;
          accountId = v === "" ? null : Number(v);
          load();
        }}
      >
        {#each app.accounts as a (a.id)}<option value={a.id}>{a.label} ({a.user_email})</option>{/each}
        <option value="">Global identity (remove repo-level settings)</option>
      </select>
    </label>

    {#if account}
      <p class="muted">
        Commits will be authored as <strong>{account.user_name} &lt;{account.user_email}&gt;</strong> and pushes will
        use {account.auth_method === "ssh" ? account.ssh_key_path : "your HTTPS credential helper"}. Each repo's own
        <code>.git/config</code> is changed; nothing global is.
      </p>
      {#if account.health?.level === "error"}
        <p class="warn-text">{account.label} can't sign in right now: {account.health.text}</p>
      {/if}
    {/if}

    {#if error}
      <p class="err">{error}</p>
    {:else if !items}
      <p class="faint">Reading each repo's config…</p>
    {:else}
      <p>
        <strong>{changing.length}</strong> will change{#if matching}, <span class="faint">{matching} already match</span>{/if}.
      </p>
      {#if changing.length}
        <button class="ghost link" onclick={() => (showDetails = !showDetails)}>
          {showDetails ? "Hide" : "Show"} the changes per repo
        </button>
        {#if showDetails}
          <div class="details">
            {#each changing as it (it.repo_id)}
              <div class="repo">
                <strong>{it.name}</strong>
                {#each it.changes as c (c.key)}
                  <div class="mono small">
                    <span class="faint">{c.key}</span>
                    <span class="old">{c.current ?? "—"}</span> → <span>{c.next ?? "(removed)"}</span>
                  </div>
                {/each}
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    {/if}

    <div class="row-flex">
      <span class="spacer"></span>
      <button onclick={onclose}>Cancel</button>
      <button class="primary" onclick={apply} disabled={busy || !items || !!error || !items.length}>
        {changing.length ? `Apply to ${changing.length} repo${changing.length === 1 ? "" : "s"}` : "Mark as active"}
      </button>
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
  .pick {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pick select {
    flex: 1;
  }
  .details {
    display: grid;
    gap: 8px;
    max-height: 260px;
    overflow: auto;
    background: var(--surface-2);
    padding: 10px;
    border-radius: 6px;
  }
  .repo {
    display: grid;
    gap: 2px;
  }
  .small {
    font-size: 11px;
    word-break: break-all;
  }
  .old {
    color: var(--text-3);
    text-decoration: line-through;
  }
  .link {
    justify-self: start;
    color: var(--accent);
    padding: 0;
    height: auto;
  }
  .warn-text {
    color: var(--warn);
  }
  .err {
    color: var(--error);
  }
</style>
