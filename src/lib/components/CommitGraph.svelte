<script lang="ts">
  import { onMount } from "svelte";
  import { api, ago, errText, type ChangedFile, type GraphRow } from "$lib/api";
  import { app } from "$lib/state.svelte";

  let { repoId, hasGithub, onloaded }: { repoId: number; hasGithub: boolean; onloaded?: () => void } = $props();

  const ROW = 26;
  const LANE = 14;
  const MAX_LANES = 24;

  let rows = $state<GraphRow[]>([]);
  let hasMore = $state(false);
  let page = $state(0);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let height = $state(500);
  let picked = $state<GraphRow | null>(null);
  let files = $state<ChangedFile[] | null>(null);

  const start = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 10));
  const end = $derived(Math.min(rows.length, Math.ceil((scrollTop + height) / ROW) + 10));
  const lanes = $derived(
    Math.min(MAX_LANES, rows.reduce((m, r) => Math.max(m, r.lane, ...r.segments.map((s) => Math.max(s.x1, s.x2))), 0) + 1),
  );
  const graphWidth = $derived(lanes * LANE + 6);

  async function loadPage(p: number) {
    if (loading) return;
    loading = true;
    try {
      const res = await api.getGraph(repoId, p);
      rows = p === 0 ? res.rows : [...rows, ...res.rows];
      hasMore = res.has_more;
      page = p;
      error = null;
      if (p === 0) onloaded?.();
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => loadPage(0));

  function onScroll() {
    scrollTop = viewport!.scrollTop;
    if (hasMore && !loading && scrollTop + height > rows.length * ROW - 40 * ROW) loadPage(page + 1);
  }

  async function pick(r: GraphRow) {
    picked = r;
    files = null;
    files = (await app.run(() => api.commitFiles(repoId, r.hash, r.parents[0] ?? null))) ?? [];
  }

  const x = (col: number) => 3 + col * LANE + LANE / 2;
  const y = (v: number) => (v * ROW) / 2;
  const color = (c: number) => `var(--lane-${c % 8})`;

  function segPath(s: { x1: number; y1: number; x2: number; y2: number }) {
    const [x1, y1, x2, y2] = [x(s.x1), y(s.y1), x(s.x2), y(s.y2)];
    if (x1 === x2) return `M${x1} ${y1}L${x2} ${y2}`;
    const my = (y1 + y2) / 2;
    return `M${x1} ${y1}C${x1} ${my} ${x2} ${my} ${x2} ${y2}`;
  }

  function refBadges(refs: string) {
    if (!refs) return [];
    return refs.split(", ").map((r) => {
      if (r.startsWith("HEAD -> ")) return { text: r.slice(8), kind: "head" };
      if (r.startsWith("tag: ")) return { text: r.slice(5), kind: "tag" };
      if (r === "HEAD") return { text: "HEAD", kind: "head" };
      return { text: r, kind: r.includes("/") ? "remote" : "branch" };
    });
  }
</script>

<div class="graph">
  {#if error}
    <p class="err">{error}</p>
  {:else if !rows.length && loading}
    <p class="faint pad">Loading history…</p>
  {:else if !rows.length}
    <p class="faint pad">No commits yet.</p>
  {/if}
  <div class="viewport" bind:this={viewport} bind:clientHeight={height} onscroll={onScroll}>
    <div style="height: {rows.length * ROW}px; position: relative;">
      {#each rows.slice(start, end) as r, i (r.hash)}
        <div
          class="row"
          class:picked={picked?.hash === r.hash}
          style="top: {(start + i) * ROW}px"
          onclick={() => pick(r)}
          onkeydown={(e) => e.key === "Enter" && pick(r)}
          role="button"
          tabindex="-1"
        >
          <svg width={graphWidth} height={ROW} aria-hidden="true">
            {#each r.segments as s}
              {#if s.x1 < MAX_LANES && s.x2 < MAX_LANES}
                <path d={segPath(s)} stroke={color(s.c)} stroke-width="1.6" fill="none" />
              {/if}
            {/each}
            {#if r.lane < MAX_LANES}
              <circle
                cx={x(r.lane)}
                cy={ROW / 2}
                r={r.parents.length > 1 ? 3.2 : 3.6}
                fill={r.parents.length > 1 ? "var(--surface)" : color(r.lane)}
                stroke={color(r.lane)}
                stroke-width="1.6"
              />
            {/if}
          </svg>
          <div class="msg truncate">
            {#each refBadges(r.refs) as b}
              <span class="badge {b.kind === 'head' ? 'accent' : ''}">{b.text}</span>
            {/each}
            <span>{r.subject}</span>
          </div>
          <span class="author truncate faint" title={r.author_email}>{r.author_name}</span>
          <span class="when faint">{ago(r.authored_at)}</span>
        </div>
      {/each}
    </div>
  </div>

  {#if picked}
    <div class="commit">
      <div class="row-flex">
        <code>{picked.hash.slice(0, 10)}</code>
        <strong class="truncate">{picked.subject}</strong>
        <span class="spacer"></span>
        {#if hasGithub}
          <button onclick={() => app.run(() => api.openOnGithub(repoId, undefined, picked!.hash))}>Open on GitHub</button>
        {/if}
        <button class="ghost" aria-label="Close" onclick={() => (picked = null)}>✕</button>
      </div>
      <div class="faint">
        {picked.author_name} &lt;{picked.author_email}&gt; · {new Date(picked.authored_at * 1000).toLocaleString()}
        {#if picked.parents.length > 1}· merge of {picked.parents.length} parents{/if}
      </div>
      <div class="files">
        {#if files === null}
          <span class="faint">Reading changed files…</span>
        {:else if files.length === 0}
          <span class="faint">No file changes.</span>
        {:else}
          {#each files as f}
            <div class="file"><span class="st st-{f.status[0]}">{f.status[0]}</span><span class="mono truncate">{f.path}</span></div>
          {/each}
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .graph {
    display: grid;
    grid-template-rows: 1fr auto;
    min-height: 0;
  }
  .pad {
    padding: 14px;
  }
  .viewport {
    overflow: auto;
    position: relative;
    min-height: 0;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 26px;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) minmax(60px, 140px) 64px;
    align-items: center;
    gap: 8px;
    padding-right: 12px;
    cursor: default;
  }
  .row:hover {
    background: var(--row-hover);
  }
  .row.picked {
    background: var(--row-selected);
  }
  svg {
    display: block;
  }
  .msg {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .msg span:last-child {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .author,
  .when {
    font-size: 12px;
  }
  .when {
    text-align: right;
  }
  .commit {
    border-top: 1px solid var(--border);
    padding: 10px 14px;
    display: grid;
    gap: 6px;
    max-height: 40%;
    overflow: hidden;
    grid-template-rows: auto auto 1fr;
  }
  .files {
    overflow: auto;
    max-height: 180px;
  }
  .file {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .st {
    width: 14px;
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 600;
  }
  .st-A {
    color: var(--ok);
  }
  .st-D {
    color: var(--error);
  }
  .st-M {
    color: var(--warn);
  }
  .err {
    color: var(--error);
    padding: 14px;
  }
</style>
