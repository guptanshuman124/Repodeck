<script lang="ts">
  import { api, checkWords, errText, keySlug, type Account, type CheckResult, type GeneratedKey } from "$lib/api";
  import { open } from "@tauri-apps/plugin-dialog";

  let {
    account = null,
    onsaved,
    oncancel,
  }: { account?: Account | null; onsaved: (id: number) => void; oncancel?: () => void } = $props();

  // The form edits a copy; the parent remounts it for a different account.
  const initial = (): Account =>
    account
      ? { ...account }
      : {
          id: null,
          label: "",
          user_name: "",
          user_email: "",
          host: "github.com",
          auth_method: "ssh",
          ssh_key_path: "",
          expected_username: "",
          owners: "",
        };
  let form = $state<Account>(initial());
  let error = $state<string | null>(null);
  let busy = $state(false);

  let newKey = $state<GeneratedKey | null>(null);
  let keyBusy = $state(false);
  let keyError = $state<string | null>(null);
  let test = $state<CheckResult | null>(null);
  let testing = $state(false);
  let copied = $state(false);

  const suggestedKey = $derived(`~/.ssh/id_ed25519_${keySlug(form.label)}`);
  const pubPicked = $derived(!!form.ssh_key_path?.trim().endsWith(".pub"));
  const isGithub = $derived((form.host || "github.com").trim().toLowerCase() === "github.com");

  async function generateKey() {
    keyBusy = true;
    keyError = null;
    test = null;
    try {
      newKey = await api.generateSshKey(form.label, form.user_email);
      form.ssh_key_path = newKey.private_path;
    } catch (e) {
      keyError = errText(e);
    } finally {
      keyBusy = false;
    }
  }

  async function copyKey() {
    if (!newKey) return;
    try {
      await navigator.clipboard.writeText(newKey.public_key);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {
      keyError = "Couldn't copy. Select the key above and copy it by hand.";
    }
  }

  async function openGithub() {
    await copyKey();
    try {
      await api.openSshKeyPage(form.host || "github.com");
    } catch (e) {
      keyError = errText(e);
    }
  }

  async function testKey() {
    if (!form.ssh_key_path?.trim()) return;
    testing = true;
    keyError = null;
    try {
      test = await api.testSshKey(form.ssh_key_path.trim(), form.host, form.expected_username?.trim() || null, form.id);
    } catch (e) {
      keyError = errText(e);
    } finally {
      testing = false;
    }
  }

  async function trustGithub() {
    testing = true;
    try {
      await api.trustGithubHost();
    } catch (e) {
      keyError = errText(e);
      testing = false;
      return;
    }
    await testKey();
  }

  async function pickKey() {
    const p = await open({ title: "Choose a private SSH key", multiple: false, directory: false });
    if (typeof p === "string") form.ssh_key_path = p;
  }

  async function save(e: Event) {
    e.preventDefault();
    busy = true;
    error = null;
    try {
      const id = await api.saveAccount(form);
      onsaved(id);
    } catch (err) {
      error = errText(err);
    } finally {
      busy = false;
    }
  }
</script>

<form onsubmit={save}>
  <div class="grid">
    <label>Label <input type="text" bind:value={form.label} placeholder="Personal, Work…" required /></label>
    <label>Host <input type="text" bind:value={form.host} placeholder="github.com" /></label>
    <label>Commit name <input type="text" bind:value={form.user_name} placeholder="Ada Lovelace" required /></label>
    <label>Commit email <input type="text" bind:value={form.user_email} placeholder="ada@example.com" required /></label>
    <label>
      Sign in with
      <select bind:value={form.auth_method}>
        <option value="ssh">SSH key</option>
        <option value="https">HTTPS (your credential helper)</option>
      </select>
    </label>
    <label>
      GitHub username <span class="faint">(optional, confirms push checks)</span>
      <input type="text" bind:value={form.expected_username} placeholder="ada-l" />
    </label>
    {#if form.auth_method === "ssh"}
      <label class="wide">
        Private key path <span class="faint">(only the path is stored, never the key)</span>
        <span class="row-flex">
          <input type="text" class="grow mono" bind:value={form.ssh_key_path} placeholder={suggestedKey} />
          <button type="button" onclick={pickKey}>Browse…</button>
          <button type="button" onclick={testKey} disabled={testing || !form.ssh_key_path?.trim()} title="Sign in to GitHub with this key">
            {testing ? "Testing…" : "Test"}
          </button>
        </span>
      </label>
      {#if pubPicked}
        <p class="wide err small">
          That's the public key. RepoDeck needs the private key: the same name without <code>.pub</code>. It switches to
          it automatically when you save, if the file is there.
        </p>
      {/if}
      <div class="wide keytools">
        <div class="row-flex">
          <button
            type="button"
            onclick={generateKey}
            disabled={keyBusy || !form.label.trim() || !form.user_email.includes("@")}
            title={form.label.trim() && form.user_email.includes("@") ? "" : "Fill in the label and email first"}
          >
            {keyBusy ? "Generating…" : "Generate new key"}
          </button>
          <span class="faint small">Creates <span class="mono">{suggestedKey}</span> (or <span class="mono">_2</span> if taken). Existing keys are never touched.</span>
        </div>

        {#if newKey}
          <div class="newkey">
            <div class="row-flex">
              <strong>New key</strong>
              <span class="faint small mono">{newKey.fingerprint}</span>
            </div>
            <textarea class="mono" readonly rows="3" onfocus={(e) => e.currentTarget.select()}>{newKey.public_key}</textarea>
            <div class="row-flex">
              <button type="button" class="primary" onclick={openGithub}>Copy and open GitHub</button>
              <button type="button" onclick={copyKey}>{copied ? "Copied" : "Copy public key"}</button>
            </div>
            <ol class="small muted">
              <li>Make sure your browser is signed in to the GitHub account this key is for.</li>
              <li>Paste the key (it's on your clipboard), keep "Authentication Key", and click <em>Add SSH key</em>.</li>
              <li>Come back and click <strong>Test</strong>.</li>
            </ol>
          </div>
        {/if}

        {#if test}
          <div class="test row-flex">
            <span class="badge {test.status === 'can_push' ? 'ok' : test.status === 'offline' || test.status === 'unknown' ? '' : 'error'}">
              {test.status === "can_push" ? "Works" : checkWords[test.status]}
            </span>
            <span class="small">{test.detail}</span>
            {#if test.status === "can_push" && test.username && !form.expected_username?.trim()}
              <button type="button" class="ghost tiny" onclick={() => (form.expected_username = test!.username!)}>
                Use @{test.username} as the GitHub username
              </button>
            {/if}
            {#if test.status === "host_untrusted" && isGithub}
              <button type="button" class="tiny" onclick={trustGithub} disabled={testing}>Trust github.com</button>
            {/if}
          </div>
        {/if}
        {#if keyError}<p class="err">{keyError}</p>{/if}
      </div>
    {/if}
    <label class="wide">
      Expected for owners <span class="faint">(optional, comma-separated orgs or users; flags repos there that commit as someone else)</span>
      <input type="text" bind:value={form.owners} placeholder="acme-corp, acme-labs" />
    </label>
  </div>
  {#if error}<p class="err">{error}</p>{/if}
  <div class="row-flex">
    <button class="primary" type="submit" disabled={busy}>{form.id ? "Save changes" : "Add account"}</button>
    {#if oncancel}<button type="button" onclick={oncancel}>Cancel</button>{/if}
  </div>
</form>

<style>
  form {
    display: grid;
    gap: 12px;
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
  .wide {
    grid-column: 1 / -1;
  }
  .grow {
    flex: 1;
  }
  .err {
    color: var(--error);
    margin: 0;
  }
  .keytools {
    display: grid;
    gap: 8px;
  }
  .small {
    font-size: 11px;
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
    flex-wrap: wrap;
    gap: 8px;
  }
  .tiny {
    height: 22px;
    padding: 0 8px;
    font-size: 11px;
  }
</style>
