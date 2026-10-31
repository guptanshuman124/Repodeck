<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import AccountWizard from "./AccountWizard.svelte";

  let { ondone }: { ondone: () => void } = $props();

  let step = $state(1);
  let adding = $state(false);
  let scanJob = $state<number | null>(null);

  const job = $derived(app.jobs.find((j) => j.id === scanJob));

  async function addFolders() {
    const picked = await open({ directory: true, multiple: true, title: "Choose folders to scan" });
    const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
    for (const p of list) await app.run(() => api.addRoot(p));
    await app.reloadRoots();
  }

  async function removeRoot(id: number) {
    await app.run(() => api.removeRoot(id));
    await app.reloadRoots();
  }

  async function startScan() {
    step = 3;
    scanJob = (await app.run(() => api.startScan())) ?? null;
  }

  async function finish() {
    await app.run(() => api.setSettings({ onboarded: "true" }));
    ondone();
  }
</script>

<div class="wrap">
  <div class="steps">
    {#each ["Folders", "Accounts", "Scan"] as label, i}
      <span class:active={step === i + 1} class:done={step > i + 1}>{i + 1}. {label}</span>
    {/each}
  </div>

  {#if step === 1}
    <section class="card">
      <h2>Where are your repos?</h2>
      <p class="muted">
        RepoDeck only looks inside folders you choose, and every folder below them. Nothing
        outside these folders is read.
      </p>
      {#if app.roots.length}
        <ul class="roots">
          {#each app.roots as r (r.id)}
            <li class="row-flex">
              <span class="mono truncate">{r.path}</span>
              <span class="spacer"></span>
              <button class="ghost" onclick={() => removeRoot(r.id)}>Remove</button>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="row-flex">
        <button onclick={addFolders}>Add folders…</button>
        <span class="spacer"></span>
        <button class="primary" disabled={!app.roots.length} onclick={() => (step = 2)}>Continue</button>
      </div>
    </section>
  {:else if step === 2}
    <section class="card">
      <h2>Add your accounts <span class="faint">(optional)</span></h2>
      <p class="muted">
        An account is a saved identity: the name and email you commit as, and the SSH key or
        HTTPS login you push with. You can link accounts to repos later.
      </p>
      {#if app.accounts.length}
        <ul class="roots">
          {#each app.accounts as a (a.id)}
            <li><strong>{a.label}</strong> <span class="muted">{a.user_email}</span></li>
          {/each}
        </ul>
      {/if}
      {#if adding}
        <AccountWizard
          onsaved={async () => {
            adding = false;
            await app.reloadAccounts();
          }}
          oncancel={() => (adding = false)}
        />
      {:else}
        <div class="row-flex">
          <button onclick={() => (adding = true)}>Add an account</button>
          <span class="spacer"></span>
          <button class="ghost" onclick={() => (step = 1)}>Back</button>
          <button class="primary" onclick={startScan}>{app.accounts.length ? "Continue" : "Skip for now"}</button>
        </div>
      {/if}
    </section>
  {:else}
    <section class="card">
      <h2>{job?.finished ? `Found ${app.repos.length} repos` : "Finding your repos…"}</h2>
      <p class="muted">{job?.label ?? "Starting…"}</p>
      {#if job && !job.finished && job.total > 0}
        <progress max={job.total} value={job.done}></progress>
      {/if}
      <ul class="found">
        {#each app.repos.slice(0, 200) as r (r.id)}
          <li class="row-flex">
            <span class="truncate">{r.name}</span>
            <span class="faint mono truncate">{r.path}</span>
          </li>
        {/each}
      </ul>
      {#if job?.finished && app.suggestions.length}
        <div class="suggestions">
          <strong>Suggested accounts</strong>
          {#each app.suggestions as s (s.accountId)}
            <span>
              {s.repoIds.length} repo{s.repoIds.length === 1 ? "" : "s"} look like
              <strong>{app.accountById.get(s.accountId)?.label}</strong> repos.
            </span>
          {/each}
          <span class="faint">You can review and apply these with one click on the next screen.</span>
        </div>
      {/if}
      <div class="row-flex">
        <span class="spacer"></span>
        <button class="primary" onclick={finish} disabled={!job?.finished && app.repos.length === 0}>
          Open RepoDeck
        </button>
      </div>
    </section>
  {/if}
</div>

<style>
  .wrap {
    height: 100vh;
    overflow: auto;
    display: grid;
    align-content: center;
    justify-content: center;
    gap: 16px;
    padding: 32px;
  }
  section {
    width: min(620px, 92vw);
    display: grid;
    gap: 12px;
  }
  p {
    margin: 0;
  }
  .steps {
    display: flex;
    gap: 18px;
    color: var(--text-3);
  }
  .steps .active {
    color: var(--accent);
    font-weight: 600;
  }
  .steps .done {
    color: var(--text-2);
  }
  .roots,
  .found {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 4px;
  }
  .found {
    max-height: 280px;
    overflow: auto;
  }
  .found li {
    gap: 12px;
  }
  .found li span:first-child {
    flex: none;
    max-width: 40%;
  }
  progress {
    width: 100%;
    accent-color: var(--accent);
  }
  .suggestions {
    display: grid;
    gap: 4px;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--accent-soft);
    font-size: 13px;
  }
</style>
