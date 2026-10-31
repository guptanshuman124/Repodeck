<script lang="ts">
  import { onMount } from "svelte";
  import { api, errText, type AppStatus } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import Unlock from "$lib/components/Unlock.svelte";
  import Setup from "$lib/components/Setup.svelte";
  import Main from "$lib/components/Main.svelte";

  let status = $state<AppStatus | null>(null);
  let bootError = $state<string | null>(null);
  let loaded = $state(false);

  async function boot() {
    try {
      status = await api.appStatus();
      if (status.db === "ready" && status.git_ok) {
        await app.loadAll();
        loaded = true;
      }
    } catch (e) {
      bootError = errText(e);
    }
  }

  onMount(() => {
    app.listen();
    boot();
  });
</script>

{#if bootError}
  <div class="gate"><div class="card"><h2>Something went wrong</h2><p class="muted">{bootError}</p></div></div>
{:else if !status}
  <div class="gate"></div>
{:else if !status.git_ok}
  <div class="gate">
    <div class="card narrow">
      <h2>Git is not installed</h2>
      <p class="muted">
        RepoDeck uses your own Git so identities resolve exactly as a real commit or push would.
        Install Git 2.26 or newer and make sure <code>git</code> is on your PATH, then reopen RepoDeck.
      </p>
      <ul class="muted">
        <li>macOS: <code>xcode-select --install</code> or <code>brew install git</code></li>
        <li>Windows: install Git for Windows from git-scm.com</li>
        <li>Linux: <code>sudo apt install git</code> (or your distro's package manager)</li>
      </ul>
      <button onclick={boot}>Check again</button>
    </div>
  </div>
{:else if status.db !== "ready"}
  <Unlock dbState={status.db} onready={boot} />
{:else if !loaded}
  <div class="gate"></div>
{:else if !status.onboarded}
  <Setup ondone={() => (status = { ...status!, onboarded: true })} />
{:else}
  {#if status.git_too_old}
    <div class="banner">
      {status.git_version} is older than 2.26, so config sources cannot be shown. Please update Git.
    </div>
  {/if}
  <Main />
{/if}

{#if app.toast}
  <div class="toast" class:error={app.toast.error} role="status">{app.toast.text}</div>
{/if}

<style>
  .gate {
    height: 100vh;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .narrow {
    max-width: 520px;
    display: grid;
    gap: 10px;
  }
  .narrow p,
  .narrow ul {
    margin: 0;
  }
  .banner {
    background: var(--warn-soft);
    color: var(--warn);
    padding: 6px 16px;
    font-size: 12px;
  }
  .toast {
    position: fixed;
    bottom: 56px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--text);
    color: var(--bg);
    padding: 8px 14px;
    border-radius: 8px;
    box-shadow: var(--shadow);
    max-width: 70vw;
    z-index: 100;
  }
  .toast.error {
    background: var(--error);
    color: #fff;
  }
</style>
