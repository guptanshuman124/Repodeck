<script lang="ts">
  import { api, errText, type DbState } from "$lib/api";

  let { dbState, onready }: { dbState: DbState; onready: () => void } = $props();

  let pass = $state("");
  let confirm = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);
  let confirmReset = $state(false);

  const firstTime = $derived(
    typeof dbState === "object" && "needs_passphrase" in dbState && dbState.needs_passphrase.first_time,
  );

  async function unlock(e: Event) {
    e.preventDefault();
    error = null;
    if (firstTime && pass !== confirm) {
      error = "The passphrases do not match.";
      return;
    }
    busy = true;
    try {
      await api.unlock(pass);
      onready();
    } catch (err) {
      error = errText(err);
    } finally {
      busy = false;
    }
  }

  async function reset() {
    busy = true;
    try {
      await api.resetData();
      onready();
    } catch (err) {
      error = errText(err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="gate">
  <div class="card box">
    {#if dbState === "key_lost"}
      <h2>RepoDeck can't open its data</h2>
      <p class="muted">
        The key for RepoDeck's encrypted database is missing from your system keychain, so the
        saved data can't be read. Your Git repos and config are not affected. Starting fresh
        deletes RepoDeck's cache and account profiles; it does not touch any repo.
      </p>
      {#if confirmReset}
        <div class="row-flex">
          <button class="primary" onclick={reset} disabled={busy}>Delete RepoDeck data and start fresh</button>
          <button onclick={() => (confirmReset = false)}>Cancel</button>
        </div>
      {:else}
        <button onclick={() => (confirmReset = true)}>Start fresh…</button>
      {/if}
    {:else if typeof dbState === "object" && "error" in dbState}
      <h2>Database error</h2>
      <p class="muted">{dbState.error.message}</p>
    {:else}
      <form onsubmit={unlock}>
        <h2>{firstTime ? "Create a passphrase" : "Unlock RepoDeck"}</h2>
        <p class="muted">
          {#if firstTime}
            This system has no keychain service, so RepoDeck encrypts its data with a passphrase
            instead. It is never stored; if you forget it you will need to start fresh.
          {:else}
            Enter your passphrase to decrypt RepoDeck's local data.
          {/if}
        </p>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" placeholder="Passphrase" bind:value={pass} autofocus />
        {#if firstTime}
          <input type="password" placeholder="Repeat passphrase" bind:value={confirm} />
        {/if}
        <button class="primary" type="submit" disabled={busy || pass.length < 8}>
          {firstTime ? "Create and continue" : "Unlock"}
        </button>
      </form>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
  </div>
</div>

<style>
  .gate {
    height: 100vh;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .box {
    width: min(460px, 100%);
    display: grid;
    gap: 12px;
  }
  form {
    display: grid;
    gap: 10px;
  }
  p {
    margin: 0;
  }
  .err {
    color: var(--error);
  }
</style>
