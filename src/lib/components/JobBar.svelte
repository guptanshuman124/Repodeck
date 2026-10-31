<script lang="ts">
  import { api } from "$lib/api";
  import { app } from "$lib/state.svelte";

  const names: Record<string, string> = {
    fetched: "fetched",
    auth_failed: "sign-in failed",
    host_untrusted: "host not trusted",
    no_remote: "no remote",
    offline: "offline",
    timed_out: "timed out",
    failed: "failed",
    cancelled: "cancelled",
    can_push: "can push",
    read_only: "read only",
    no_access: "no access",
    wrong_account: "wrong account",
    unknown: "unknown",
    repos: "repos",
    cloned: "cloned",
    can_read: "can read",
  };

  function summary(s: Record<string, number>) {
    return Object.entries(s)
      .sort((a, b) => b[1] - a[1])
      .map(([k, v]) => `${v} ${names[k] ?? k}`)
      .join(" · ");
  }
</script>

{#if app.jobs.length}
  <footer>
    {#each app.jobs as j (j.id)}
      <div class="job">
        <span class="kind">{j.kind === "scan" ? "Scan" : j.kind === "fetch" ? "Fetch" : j.kind === "clone" ? "Clone" : "Access scan"}</span>
        {#if !j.finished}
          {#if j.total > 0}
            <progress max={j.total} value={j.done}></progress>
          {:else}
            <progress></progress>
          {/if}
          <span class="label truncate">{j.label}</span>
          <span class="spacer"></span>
          <button onclick={() => api.cancelJob(j.id)}>Cancel</button>
        {:else}
          <span class="label truncate">{j.label}{Object.keys(j.summary).length ? `: ${summary(j.summary)}` : ""}</span>
          <span class="spacer"></span>
          {#if j.failed.length}
            <button onclick={() => { app.dismissJob(j.id); app.run(() => api.startFetch(j.failed)); }}>
              Retry {j.failed.length} failed
            </button>
          {/if}
          <button class="ghost" onclick={() => app.dismissJob(j.id)}>Dismiss</button>
        {/if}
      </div>
    {/each}
  </footer>
{/if}

<style>
  footer {
    border-top: 1px solid var(--border);
    background: var(--surface);
    display: grid;
  }
  .job {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 14px;
    min-width: 0;
  }
  .job + .job {
    border-top: 1px solid var(--border);
  }
  .kind {
    font-weight: 600;
    width: 86px;
    flex: none;
  }
  progress {
    width: 160px;
    flex: none;
    accent-color: var(--accent);
  }
  .label {
    color: var(--text-2);
  }
</style>
