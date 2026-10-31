<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    api,
    checkWords,
    errText,
    keySlug,
    type Account,
    type CheckResult,
    type GeneratedKey,
    type SshKeyInfo,
  } from "$lib/api";
  import { app } from "$lib/state.svelte";

  let { onsaved, oncancel }: { onsaved: (id: number) => void; oncancel?: () => void } = $props();

  let step = $state(1);
  let form = $state<Account>({
    id: null,
    label: "",
    user_name: "",
    user_email: "",
    host: "github.com",
    auth_method: "ssh",
    ssh_key_path: "",
    expected_username: "",
    owners: "",
  });
  let showAdvanced = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);

  // Step 2: sign-in
  let keys = $state<SshKeyInfo[] | null>(null);
  let newKey = $state<GeneratedKey | null>(null);
  let test = $state<CheckResult | null>(null);
  let testing = $state(false);
  let polling = $state(false);
  let otherPath = $state(false);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;

  // Step 3: where it applies
  let applyNow = $state(true);

  const suggestedName = $derived(`id_ed25519_${keySlug(form.label)}`);
  const step1Ok = $derived(!!form.label.trim() && !!form.user_name.trim() && form.user_email.includes("@"));
  const keyPath = $derived(form.ssh_key_path?.trim() ?? "");
  const pubPicked = $derived(keyPath.endsWith(".pub"));
  const signedIn = $derived(test?.status === "can_push");
  const step2Ok = $derived(form.auth_method === "https" || (!!keyPath && !pubPicked));

  /** Owners of repos that already commit with this email: a good guess for "expected for". */
  const guessedOwners = $derived.by(() => {
    const email = form.user_email.trim().toLowerCase();
    if (!email) return [];
    const owners = new Set<string>();
    for (const r of app.repos) {
      if (r.effective_email?.toLowerCase() === email && r.remote?.owner) owners.add(r.remote.owner);
    }
    return [...owners].sort();
  });
  const ownerList = $derived(
    (form.owners ?? "")
      .split(",")
      .map((o) => o.trim().toLowerCase())
      .filter(Boolean),
  );
  /** Repos under those owners: what "Apply to N repos now" would switch. */
  const matchingRepos = $derived(
    app.repos.filter((r) => !r.is_bare && r.remote?.owner && ownerList.includes(r.remote.owner.toLowerCase())),
  );

  onMount(loadKeys);
  onDestroy(stopPolling);

  async function loadKeys() {
    try {
      keys = await api.listSshKeys();
    } catch (e) {
      keys = [];
      error = errText(e);
    }
  }

  function goStep3() {
    if (!form.owners?.trim() && guessedOwners.length) form.owners = guessedOwners.join(", ");
    step = 3;
  }

  async function pickKey(path: string) {
    otherPath = false;
    form.ssh_key_path = path;
    newKey = null;
    stopPolling();
    await runTest();
  }

  async function browse() {
    const p = await open({ title: "Choose a private SSH key", multiple: false, directory: false });
    if (typeof p === "string") {
      form.ssh_key_path = p;
      await runTest();
    }
  }

  async function runTest() {
    if (!keyPath || pubPicked) return;
    testing = true;
    try {
      test = await api.testSshKey(keyPath, form.host, form.expected_username?.trim() || null);
      if (test.status === "can_push" && test.username) form.expected_username = test.username;
    } catch (e) {
      error = errText(e);
    } finally {
      testing = false;
    }
  }

  async function generate() {
    busy = true;
    error = null;
    test = null;
    try {
      newKey = await api.generateSshKey(form.label, form.user_email);
      form.ssh_key_path = newKey.private_path;
      await loadKeys();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function copyAndOpen() {
    if (!newKey) return;
    try {
      await navigator.clipboard.writeText(newKey.public_key);
    } catch {
      error = "Couldn't copy the key. Select it above and copy it by hand.";
    }
    try {
      await api.openSshKeyPage(form.host || "github.com");
    } catch (e) {
      error = errText(e);
    }
    startPolling();
  }

  /** Re-test every 5 s for up to 2 minutes while the user adds the key on GitHub. */
  function startPolling() {
    stopPolling();
    polling = true;
    const until = Date.now() + 120_000;
    const tick = async () => {
      await runTest();
      if (signedIn || Date.now() > until || step !== 2) {
        polling = false;
        return;
      }
      pollTimer = setTimeout(tick, 5000);
    };
    pollTimer = setTimeout(tick, 5000);
  }

  function stopPolling() {
    if (pollTimer) clearTimeout(pollTimer);
    pollTimer = null;
    polling = false;
  }

  async function trustGithub() {
    testing = true;
    try {
      await api.trustGithubHost();
    } catch (e) {
      error = errText(e);
    }
    testing = false;
    await runTest();
  }

  async function save() {
    busy = true;
    error = null;
    try {
      const id = await api.saveAccount(form);
      // Record the sign-in test against the saved account so its health shows right away.
      if (form.auth_method === "ssh" && signedIn) {
        api.testSshKey(keyPath, form.host, form.expected_username?.trim() || null, id).catch(() => {});
      }
      await app.reloadAccounts();
      const ids = matchingRepos.filter((r) => r.active_account_id !== id).map((r) => r.id);
      onsaved(id);
      if (applyNow && ids.length) app.bulkReq = { repoIds: ids, accountId: id };
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function badgeClass(s: string) {
    return s === "can_push" ? "ok" : s === "offline" || s === "unknown" ? "" : "error";
  }
</script>

<div class="wizard">
  <ol class="steps">
    {#each ["Who you commit as", "How you sign in", "Where it applies"] as label, i}
      <li class:active={step === i + 1} class:done={step > i + 1}>{label}</li>
    {/each}
  </ol>

  {#if step === 1}
    <div class="grid">
      <label>Label <input type="text" bind:value={form.label} placeholder="Personal, Work, Acme…" /></label>
      <label>Commit name <input type="text" bind:value={form.user_name} placeholder="Ada Lovelace" /></label>
      <label class="wide">
        Commit email
        <input type="email" bind:value={form.user_email} placeholder="ada@example.com" />
        <span class="faint small">Use the email GitHub knows for this account, so commits are linked to it.</span>
      </label>
    </div>
    <button class="ghost link" onclick={() => (showAdvanced = !showAdvanced)}>
      {showAdvanced ? "Hide" : "Show"} advanced
    </button>
    {#if showAdvanced}
      <label class="narrow">Host <input type="text" bind:value={form.host} placeholder="github.com" /></label>
    {/if}
  {:else if step === 2}
    <div class="seg" role="radiogroup" aria-label="Sign in with">
      <button class:on={form.auth_method === "ssh"} onclick={() => (form.auth_method = "ssh")}>SSH key</button>
      <button class:on={form.auth_method === "https"} onclick={() => (form.auth_method = "https")}>HTTPS</button>
    </div>

    {#if form.auth_method === "https"}
      <p class="muted">
        Git will use your existing credential helper (for example the macOS keychain or Git Credential Manager) and
        pick this account's login by its username.
      </p>
      <label class="narrow">
        GitHub username
        <input type="text" bind:value={form.expected_username} placeholder="ada-l" />
      </label>
    {:else}
      <p class="muted">Each GitHub account needs its own key. Pick one that's already on this Mac, or create one.</p>

      <div class="keys">
        <button class="key new" class:on={!!newKey} onclick={generate} disabled={busy}>
          <span class="title">{busy ? "Creating…" : "Create a new key"}</span>
          <span class="mono faint small">{newKey ? newKey.private_path : `~/.ssh/${suggestedName}`}</span>
          <span class="faint small">Recommended. Never replaces an existing key.</span>
        </button>
        {#each keys ?? [] as k (k.path)}
          <button class="key" class:on={keyPath === k.path && !newKey} onclick={() => pickKey(k.path)}>
            <span class="title mono">{k.name}</span>
            <span class="faint small">{k.comment ?? "no comment"}{k.kind ? ` · ${k.kind}` : ""}</span>
            <span class="small">
              {#if k.used_by.length}<span class="badge">used by {k.used_by.join(", ")}</span>{/if}
              {#if k.in_agent}<span class="badge">in ssh-agent</span>{/if}
            </span>
          </button>
        {/each}
      </div>
      {#if keys && !keys.length}
        <p class="faint small">No keys in ~/.ssh yet.</p>
      {/if}

      {#if otherPath}
        <span class="row-flex">
          <input type="text" class="grow mono" bind:value={form.ssh_key_path} placeholder="~/.ssh/{suggestedName}" />
          <button onclick={browse}>Browse…</button>
          <button onclick={runTest} disabled={!keyPath || pubPicked || testing}>Test</button>
        </span>
        {#if pubPicked}
          <p class="err small">That's the public key. Choose the file without <code>.pub</code>.</p>
        {/if}
      {:else}
        <button class="ghost link" onclick={() => (otherPath = true)}>Use a key file somewhere else…</button>
      {/if}

      {#if newKey}
        <div class="newkey">
          <div class="row-flex">
            <strong>Add this key to GitHub</strong>
            <span class="faint small mono">{newKey.fingerprint}</span>
          </div>
          <textarea class="mono" readonly rows="3" onfocus={(e) => e.currentTarget.select()}>{newKey.public_key}</textarea>
          <ol class="small muted">
            <li>Make sure your browser is signed in to the GitHub account for <strong>{form.label}</strong>.</li>
            <li>Click below, paste the key, keep "Authentication Key", and click <em>Add SSH key</em>.</li>
            <li>RepoDeck checks every few seconds and continues once GitHub accepts it.</li>
          </ol>
          <div class="row-flex">
            <button class="primary" onclick={copyAndOpen}>Copy key and open GitHub</button>
            {#if polling}<span class="faint small">Waiting for GitHub…</span>{/if}
          </div>
        </div>
      {/if}

      {#if testing && !polling}
        <p class="faint small">Signing in to GitHub with this key…</p>
      {:else if test}
        <div class="test">
          <span class="badge {badgeClass(test.status)}">{test.status === "can_push" ? "Works" : checkWords[test.status]}</span>
          <span class="small">
            {#if signedIn}Signs in as <strong>@{test.username}</strong>.{:else if test.status === "auth_failed" && newKey}GitHub doesn't know this key yet.{:else}{test.detail}{/if}
          </span>
          {#if test.status === "host_untrusted"}
            <button class="tiny" onclick={trustGithub}>Trust github.com</button>
          {/if}
        </div>
      {/if}
    {/if}
  {:else}
    <p class="muted">
      Tell RepoDeck which GitHub owners (users or organisations) belong to <strong>{form.label}</strong>. Repos under
      them are expected to use this account, and RepoDeck points out any that don't.
    </p>
    <label>
      Owners <span class="faint">(comma-separated, optional)</span>
      <input type="text" bind:value={form.owners} placeholder="acme-corp, acme-labs" />
    </label>
    {#if guessedOwners.length}
      <p class="faint small">Suggested from repos that already commit as {form.user_email}: {guessedOwners.join(", ")}</p>
    {/if}
    {#if matchingRepos.length}
      <label class="check">
        <input type="checkbox" bind:checked={applyNow} />
        Switch the {matchingRepos.length} matching repo{matchingRepos.length === 1 ? "" : "s"} to {form.label} after saving
        (you'll see a preview first)
      </label>
    {/if}
  {/if}

  {#if error}<p class="err">{error}</p>{/if}

  <div class="row-flex">
    {#if oncancel}<button class="ghost" onclick={oncancel}>Cancel</button>{/if}
    <span class="spacer"></span>
    {#if step > 1}<button onclick={() => (step -= 1)}>Back</button>{/if}
    {#if step === 1}
      <button class="primary" disabled={!step1Ok} onclick={() => (step = 2)}>Continue</button>
    {:else if step === 2}
      {#if form.auth_method === "ssh" && keyPath && !signedIn && !pubPicked}
        <span class="faint small">You can continue now and test later.</span>
      {/if}
      <button class="primary" disabled={!step2Ok} onclick={goStep3}>Continue</button>
    {:else}
      <button class="primary" disabled={busy} onclick={save}>Save account</button>
    {/if}
  </div>
</div>

<style>
  .wizard {
    display: grid;
    gap: 12px;
  }
  .steps {
    display: flex;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
    counter-reset: step;
    font-size: 12px;
  }
  .steps li {
    counter-increment: step;
    flex: 1;
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--surface-2);
    color: var(--text-3);
  }
  .steps li::before {
    content: counter(step) ". ";
  }
  .steps li.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }
  .steps li.done {
    color: var(--text-2);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 14px;
  }
  label {
    display: grid;
    gap: 4px;
    font-size: 12px;
  }
  label.check {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .wide {
    grid-column: 1 / -1;
  }
  .narrow {
    max-width: 320px;
  }
  .small {
    font-size: 11px;
  }
  p {
    margin: 0;
  }
  .link {
    justify-self: start;
    color: var(--accent);
    padding: 0;
    height: auto;
  }
  .seg {
    display: inline-flex;
    justify-self: start;
    border: 1px solid var(--border);
    border-radius: 7px;
    overflow: hidden;
  }
  .seg button {
    border: none;
    border-radius: 0;
    background: transparent;
  }
  .seg button.on {
    background: var(--accent);
    color: #fff;
  }
  .keys {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 8px;
  }
  .key {
    display: grid;
    justify-items: start;
    gap: 3px;
    height: auto;
    padding: 10px 12px;
    text-align: left;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
  }
  .key.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .key.new {
    border-style: dashed;
  }
  .key .title {
    font-weight: 600;
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .newkey {
    display: grid;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }
  .newkey textarea {
    width: 100%;
    resize: none;
    font-size: 11px;
    word-break: break-all;
  }
  .newkey ol {
    margin: 0;
    padding-left: 18px;
    display: grid;
    gap: 2px;
  }
  .test {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .tiny {
    height: 22px;
    padding: 0 8px;
    font-size: 11px;
  }
  .err {
    color: var(--error);
  }
</style>
