<script lang="ts">
  import type { RepoRow } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import { terms } from "$lib/terminal.svelte";
  import { untrack } from "svelte";

  let { repo }: { repo: RepoRow } = $props();

  // RepoDetail is keyed by repo id, so this pane never changes repos.
  // svelte-ignore state_referenced_locally
  const session = terms.session(repo.id);
  let box: HTMLDivElement;

  const account = $derived(repo.active_account_id != null ? app.accountById.get(repo.active_account_id) : null);
  const where = $derived(repo.path.replace(/^(\/(Users|home)\/[^/]+|[A-Za-z]:[\\/]Users[\\/][^\\/]+)/, "~"));

  function fit() {
    if (!box?.isConnected || box.clientWidth === 0) return;
    try {
      session.fit.fit();
    } catch {
      // Not measurable yet (hidden); the next resize fits it.
    }
  }

  $effect(() => {
    box.appendChild(session.host);
    if (!session.opened) {
      session.term.open(session.host);
      session.opened = true;
    }
    fit();
    // Start once, on first show; later state changes must not re-run this mount effect.
    untrack(() => {
      if (!session.running && session.exited == null) session.start();
    });
    const ro = new ResizeObserver(() => fit());
    ro.observe(box);
    return () => {
      ro.disconnect();
      session.host.remove();
    };
  });

  // Focus when shown, and whenever something asks for it (T, ⌘`, Restart).
  $effect(() => {
    void terms.focusTick;
    queueMicrotask(() => session.term.focus());
  });
</script>

<div class="pane">
  <div class="bar">
    <span class="state" class:live={session.running} class:dead={session.exited != null}></span>
    <span class="mono truncate where" title={repo.path}>{where}</span>
    {#if repo.head_branch}<span class="badge">{repo.head_branch}</span>{/if}
    {#if session.exited != null}
      <span class="faint small">Shell exited{session.exited >= 0 ? ` (code ${session.exited})` : ""}</span>
    {:else if repo.effective_email}
      <span
        class="who truncate small"
        title="git commits here as {repo.effective_name ?? ''} <{repo.effective_email}>"
      >
        commits as <strong>{account?.label ?? repo.effective_name ?? ""}</strong>
        <span class="faint">&lt;{repo.effective_email}&gt;</span>
      </span>
    {/if}
    <span class="spacer"></span>
    <button class="ghost icon" title="Clear (⌘K)" aria-label="Clear" onclick={() => { terms.clear(repo.id); session.term.focus(); }}>⌫</button>
    <button class="ghost icon" title="Restart shell" aria-label="Restart shell" onclick={() => terms.restart(repo.id)}>⟳</button>
    <button class="ghost icon" title="End shell and close the terminal" aria-label="Close terminal" onclick={() => terms.close(repo.id)}>✕</button>
  </div>
  <div class="screen" bind:this={box}></div>
</div>

<style>
  .pane {
    display: grid;
    grid-template-rows: auto 1fr;
    min-height: 0;
    min-width: 0;
    border-top: 1px solid var(--border);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px 4px 14px;
    border-bottom: 1px solid var(--border);
    min-width: 0;
    font-size: 12px;
  }
  .where {
    font-size: 11px;
    color: var(--text-2);
    min-width: 0;
    flex-shrink: 1;
  }
  .who {
    color: var(--text-2);
    min-width: 0;
    flex-shrink: 2;
  }
  .who strong {
    color: var(--text);
    font-weight: 600;
  }
  .small {
    font-size: 11.5px;
  }
  .spacer {
    flex: 1;
  }
  .state {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--text-3);
  }
  .state.live {
    background: var(--ok);
  }
  .state.dead {
    background: var(--warn);
  }
  .icon {
    width: 26px;
    height: 24px;
    padding: 0;
    color: var(--text-2);
    flex: none;
  }
  .screen {
    min-height: 0;
    min-width: 0;
    padding: 6px 4px 4px 12px;
    background: var(--surface);
    overflow: hidden;
  }
  .screen :global(.term-host) {
    width: 100%;
    height: 100%;
  }
  .screen :global(.xterm-viewport) {
    background: var(--surface) !important;
  }
</style>
