<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, ago, type ScanRoot } from "$lib/api";
  import { app } from "$lib/state.svelte";

  let excludes = $state("");
  let s = $state<Record<string, string>>({ ...app.settings });
  let confirmReset = $state(false);

  onMount(async () => {
    excludes = ((await app.run(() => api.getExcludes())) ?? []).join("\n");
  });

  async function addFolders() {
    const picked = await open({ directory: true, multiple: true, title: "Choose folders to scan" });
    const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
    const ids: number[] = [];
    for (const p of list) {
      const id = await app.run(() => api.addRoot(p));
      if (id != null) ids.push(id);
    }
    await app.reloadRoots();
    if (ids.length) app.run(() => api.startScan(ids));
  }

  async function saveRoot(r: ScanRoot) {
    await app.run(() => api.updateRoot(r));
    await app.reloadRoots();
  }

  async function removeRoot(id: number) {
    await app.run(() => api.removeRoot(id));
    await app.reloadRoots();
    await app.reloadRepos();
  }

  async function saveExcludes() {
    const ok = await app.run(() => api.setExcludes(excludes.split("\n")));
    if (ok !== undefined) app.notify("Excludes saved. Rescan to apply them.");
  }

  async function saveSetting(key: string, value: string) {
    s[key] = value;
    await app.run(() => api.setSettings({ [key]: value }));
    app.settings = { ...app.settings, [key]: value };
  }

  async function reset() {
    await app.run(() => api.resetData());
    location.reload();
  }
</script>

<div class="page">
  <section>
    <div class="row-flex">
      <h2>Scan folders</h2>
      <span class="spacer"></span>
      <button onclick={addFolders}>Add folders…</button>
    </div>
    <p class="muted">Only these folders and the folders inside them are scanned. Symlinks are not followed unless you allow it.</p>
    <div class="roots">
      {#each app.roots as r (r.id)}
        <div class="root card">
          <div class="row-flex">
            <span class="mono truncate grow" title={r.path}>{r.path}</span>
            <span class="faint small">scanned {ago(r.last_scanned_at)}</span>
            <button onclick={() => app.run(() => api.startScan([r.id]))}>Rescan</button>
            <button class="ghost danger" onclick={() => removeRoot(r.id)}>Remove</button>
          </div>
          <div class="opts">
            <label><input type="checkbox" checked={r.enabled} onchange={(e) => saveRoot({ ...r, enabled: e.currentTarget.checked })} /> Enabled</label>
            <label>
              Max depth
              <input type="number" min="1" max="32" value={r.max_depth} onchange={(e) => saveRoot({ ...r, max_depth: Number(e.currentTarget.value) })} />
            </label>
            <label title="Faster on huge trees, but misses repos nested inside other repos">
              <input type="checkbox" checked={r.stop_at_repo} onchange={(e) => saveRoot({ ...r, stop_at_repo: e.currentTarget.checked })} /> Stop at first repo
            </label>
            <label>
              <input type="checkbox" checked={r.follow_symlinks} onchange={(e) => saveRoot({ ...r, follow_symlinks: e.currentTarget.checked })} /> Follow symlinks
            </label>
          </div>
        </div>
      {:else}
        <p class="faint">No folders yet.</p>
      {/each}
    </div>
  </section>

  <section>
    <h2>Skip folders</h2>
    <p class="muted">
      One pattern per line. A plain name (<code>node_modules</code>) skips that folder anywhere;
      a path (<code>~/Library</code>, <code>**/AppData</code>) skips that location. Gitignore files are not used, because nested repos are often ignored by their parent.
    </p>
    <textarea rows="8" bind:value={excludes}></textarea>
    <div><button onclick={saveExcludes}>Save patterns</button></div>
  </section>

  <section>
    <h2>Editor</h2>
    <label class="field">
      Command used by “Open in VS Code”
      <input type="text" class="mono" value={s.editor} onchange={(e) => saveSetting("editor", e.currentTarget.value.trim() || "code")} />
      <span class="faint small">For example <code>code</code>, <code>code-insiders</code> or <code>cursor</code>. The repo path is added at the end.</span>
    </label>
  </section>

  <section>
    <h2>Refresh</h2>
    <div class="grid">
      <label class="field">
        Look for new repos every (hours, 0 = never)
        <input type="number" min="0" value={s.rescan_hours} onchange={(e) => saveSetting("rescan_hours", e.currentTarget.value)} />
      </label>
      <label class="field">
        Poll for changes every (seconds, used above the watch limit)
        <input type="number" min="10" value={s.poll_interval_secs} onchange={(e) => saveSetting("poll_interval_secs", e.currentTarget.value)} />
      </label>
      <label class="field">
        Watch files for up to this many repos
        <input type="number" min="0" value={s.watch_threshold} onchange={(e) => saveSetting("watch_threshold", e.currentTarget.value)} />
      </label>
      <label class="field">
        Fetch this many repos at once
        <select value={s.fetch_concurrency} onchange={(e) => saveSetting("fetch_concurrency", e.currentTarget.value)}>
          <option value="auto">Automatic (2 or 4)</option>
          {#each [1, 2, 4, 6, 8] as n}<option value={String(n)}>{n}</option>{/each}
        </select>
      </label>
    </div>
    <label><input type="checkbox" checked={s.fetch_prune === "true"} onchange={(e) => saveSetting("fetch_prune", String(e.currentTarget.checked))} /> Prune deleted remote branches when fetching</label>
    <label><input type="checkbox" checked={s.battery_saver === "true"} onchange={(e) => saveSetting("battery_saver", String(e.currentTarget.checked))} /> Battery saver: pause periodic rescans, poll less often, use two threads</label>
  </section>

  <section>
    <h2>Commit graph</h2>
    <div class="grid">
      <label class="field">
        Load at most this many commits per repo
        <input type="number" min="500" step="500" value={s.max_commits} onchange={(e) => saveSetting("max_commits", e.currentTarget.value)} />
      </label>
      <label class="field">
        Call a branch stale after (days)
        <input type="number" min="1" value={s.stale_branch_days} onchange={(e) => saveSetting("stale_branch_days", e.currentTarget.value)} />
      </label>
    </div>
    <div><button onclick={() => app.run(async () => { await api.clearGraphCache(); app.notify("Graph cache cleared"); })}>Clear graph cache</button></div>
  </section>

  <section>
    <h2>Data</h2>
    <p class="muted">
      Everything RepoDeck stores is in one encrypted database on this machine, with the key in your
      system keychain. Nothing is sent anywhere. Deleting it removes scan folders, accounts and cached
      results; your repos and Git config are not touched.
    </p>
    {#if confirmReset}
      <div class="row-flex">
        <button class="danger" onclick={reset}>Delete all RepoDeck data</button>
        <button onclick={() => (confirmReset = false)}>Cancel</button>
      </div>
    {:else}
      <div><button class="danger" onclick={() => (confirmReset = true)}>Delete all RepoDeck data…</button></div>
    {/if}
  </section>
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
  section {
    display: grid;
    gap: 10px;
    max-width: 860px;
  }
  p {
    margin: 0;
  }
  .small {
    font-size: 11px;
  }
  .roots {
    display: grid;
    gap: 8px;
  }
  .root {
    display: grid;
    gap: 8px;
    padding: 10px 14px;
  }
  .grow {
    flex: 1;
  }
  .opts {
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
    align-items: center;
  }
  .opts label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .opts input[type="number"] {
    width: 64px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px 18px;
  }
  .field {
    display: grid;
    gap: 4px;
  }
  .field input[type="number"] {
    width: 120px;
  }
  .field select {
    width: 200px;
  }
</style>
