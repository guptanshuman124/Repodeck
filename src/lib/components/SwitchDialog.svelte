<script lang="ts">
  import { onMount } from "svelte";
  import { api, errText, type Preview } from "$lib/api";
  import { app } from "$lib/state.svelte";

  let { req, onclose }: { req: { repoId: number; accountId: number | null }; onclose: () => void } = $props();

  let preview = $state<Preview | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let showCommands = $state(false);

  const repo = $derived(app.repoById.get(req.repoId));
  const account = $derived(req.accountId != null ? app.accountById.get(req.accountId) : null);

  onMount(async () => {
    try {
      preview = await api.previewSwitch(req.repoId, req.accountId);
    } catch (e) {
      error = errText(e);
    }
  });

  async function confirm() {
    if (!preview) return;
    busy = true;
    try {
      const row = await api.applySwitch(req.repoId, req.accountId, preview.changes);
      app.upsertRow(row);
      app.notify(account ? `${repo?.name} now commits as ${account.label}` : `${repo?.name} now uses the global identity`);
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
    <h2>
      {#if account}Switch {repo?.name} to {account.label}?{:else}Use the global identity in {repo?.name}?{/if}
    </h2>
    {#if account}
      <p class="muted">
        Future commits will be authored as <strong>{account.user_name} &lt;{account.user_email}&gt;</strong>
        and pushes will use {account.auth_method === "ssh" ? "its SSH key" : "your HTTPS credential helper"}.
        Existing commits keep their original author. The remote URL is not changed.
      </p>
    {:else}
      <p class="muted">
        RepoDeck removes the repo-level identity so this repo inherits from your global Git config again.
        Linked accounts stay listed for later.
      </p>
    {/if}

    {#if error}
      <p class="err">{error}</p>
    {:else if !preview}
      <p class="faint">Reading current config…</p>
    {:else if preview.changes.length === 0}
      <p class="faint">The repo's config already matches. Nothing will be written.</p>
    {:else}
      <div class="file mono faint">{preview.config_file}</div>
      <table>
        <thead><tr><th>Setting</th><th>Now</th><th>After</th></tr></thead>
        <tbody>
          {#each preview.changes as c (c.key)}
            <tr>
              <td class="mono">{c.key}</td>
              <td class="mono old">{c.current ?? "—"}</td>
              <td class="mono new">{c.next ?? "(removed)"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      <button class="ghost link" onclick={() => (showCommands = !showCommands)}>
        {showCommands ? "Hide" : "Show"} the exact commands
      </button>
      {#if showCommands}
        <pre class="mono">{preview.commands.join("\n")}</pre>
      {/if}
    {/if}

    <div class="row-flex">
      <span class="spacer"></span>
      <button onclick={onclose}>Cancel</button>
      <button class="primary" onclick={confirm} disabled={busy || !preview || !!error}>
        {preview?.changes.length ? "Write config" : "Mark as active"}
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
    display: grid;
    gap: 12px;
    box-shadow: var(--shadow);
  }
  p {
    margin: 0;
  }
  .file {
    font-size: 11px;
  }
  td.old {
    color: var(--text-3);
    text-decoration: line-through;
    text-decoration-color: var(--border-strong);
  }
  td.new {
    color: var(--text);
  }
  td {
    word-break: break-all;
  }
  .link {
    justify-self: start;
    color: var(--accent);
    padding: 0;
    height: auto;
  }
  pre {
    margin: 0;
    background: var(--surface-2);
    padding: 10px;
    border-radius: 6px;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .err {
    color: var(--error);
  }
</style>
