// Built-in terminal sessions, one per repo. They live here, outside the components, so a shell
// and its scrollback survive switching repos, closing the detail panel or changing screens.
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { api, errText } from "./api";

const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);

// ANSI colors that stay readable on the app's own light and dark surfaces.
const LIGHT = {
  black: "#1d1d1b", red: "#c43a31", green: "#2f8a4c", yellow: "#9a6700", blue: "#3563d9",
  magenta: "#8e44ad", cyan: "#13879a", white: "#8d8d87",
  brightBlack: "#5e5e59", brightRed: "#d9534a", brightGreen: "#3aa45e", brightYellow: "#b67d00",
  brightBlue: "#4a78ea", brightMagenta: "#a35bc4", brightCyan: "#1aa0b6", brightWhite: "#b4b4ae",
};
const DARK = {
  black: "#3d3d42", red: "#ef7a70", green: "#6cc98a", yellow: "#e5c07b", blue: "#7c9cf5",
  magenta: "#c08ae0", cyan: "#5cc4d4", white: "#d4d4d0",
  brightBlack: "#75756f", brightRed: "#ff958c", brightGreen: "#8ee0a8", brightYellow: "#f0d49a",
  brightBlue: "#9db5f8", brightMagenta: "#d4a8ec", brightCyan: "#82d8e4", brightWhite: "#ffffff",
};

export function themeFromCss(): ITheme {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  return {
    ...(dark ? DARK : LIGHT),
    background: v("--surface"),
    foreground: v("--text"),
    cursor: v("--accent"),
    cursorAccent: v("--surface"),
    selectionBackground: dark ? "#3a4a78" : "#c9d7f8",
  };
}

export class TermSession {
  /** PTY session id while a shell is running. */
  id = $state<number | null>(null);
  /** Exit code once the shell has ended. */
  exited = $state<number | null>(null);
  starting = $state(false);
  readonly term: Terminal;
  readonly fit = new FitAddon();
  /** The element xterm renders into; moved between mounts so nothing is redrawn. */
  readonly host = document.createElement("div");
  opened = false;

  constructor(readonly repoId: number) {
    this.host.className = "term-host";
    this.term = new Terminal({
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--mono").trim() || "monospace",
      fontSize: 12,
      lineHeight: 1.15,
      scrollback: 10000,
      cursorBlink: true,
      cursorStyle: "block",
      allowProposedApi: false,
      macOptionIsMeta: true,
      theme: themeFromCss(),
    });
    this.term.loadAddon(this.fit);
    this.term.onData((d) => {
      if (this.exited != null) {
        if (d === "\r") terms.restart(this.repoId);
        return;
      }
      if (this.id != null) api.terminalWrite(this.id, d).catch(() => {});
    });
    this.term.onResize(({ cols, rows }) => {
      if (this.id != null) api.terminalResize(this.id, cols, rows).catch(() => {});
    });
    this.term.attachCustomKeyEventHandler((e) => this.onKey(e));
  }

  get running() {
    return this.id != null && this.exited == null;
  }

  /** Copy/paste/clear the way Terminal.app (macOS) and VS Code (Windows/Linux) do. */
  private onKey(e: KeyboardEvent): boolean {
    if (e.type !== "keydown") return true;
    const k = e.key.toLowerCase();
    // ⌘` / Ctrl+` belongs to the app (toggles the terminal).
    if ((e.metaKey || e.ctrlKey) && e.key === "`") return false;
    if (isMac && e.metaKey) {
      if (k === "c") {
        if (this.term.hasSelection()) return true; // native copy
        if (this.id != null) api.terminalWrite(this.id, "\x03").catch(() => {});
        return false;
      }
      if (k === "k") {
        this.term.clear();
        return false;
      }
      if (k === "a") {
        this.term.selectAll();
        return false;
      }
      return true; // ⌘V and the rest: native handling (paste fires an event xterm reads)
    }
    if (!isMac && e.ctrlKey) {
      if (k === "c" && (e.shiftKey || this.term.hasSelection())) {
        if (this.term.hasSelection()) navigator.clipboard.writeText(this.term.getSelection()).catch(() => {});
        this.term.clearSelection();
        return false;
      }
      // Ctrl+V / Ctrl+Shift+V: let the browser fire a paste event instead of sending ^V.
      if (k === "v") return false;
    }
    return true;
  }

  async start() {
    if (this.starting || this.running) return;
    this.starting = true;
    this.exited = null;
    try {
      const id = await api.terminalOpen(this.repoId, this.term.cols, this.term.rows, (b) => this.term.write(b));
      this.id = id;
      terms.byPty.set(id, this);
    } catch (e) {
      this.exited = -1;
      this.term.write(`\r\n\x1b[31m${errText(e)}\x1b[0m\r\n\x1b[2mPress Enter to try again.\x1b[0m\r\n`);
    } finally {
      this.starting = false;
    }
  }

  dispose() {
    if (this.id != null) {
      terms.byPty.delete(this.id);
      api.terminalClose(this.id).catch(() => {});
    }
    this.id = null;
    this.term.dispose();
    this.host.remove();
  }
}

class Terms {
  sessions = new SvelteMap<number, TermSession>();
  /** Repos whose detail panel shows the terminal instead of the tabs. */
  shown = new SvelteSet<number>();
  byPty = new Map<number, TermSession>();
  /** Bumped to ask the visible pane to focus its terminal. */
  focusTick = $state(0);

  isShown(repoId: number) {
    return this.shown.has(repoId);
  }

  isRunning(repoId: number) {
    return this.sessions.get(repoId)?.running ?? false;
  }

  /** The session for a repo, created on first use. Starting the shell waits for the pane to size it. */
  session(repoId: number) {
    let s = this.sessions.get(repoId);
    if (!s) {
      s = new TermSession(repoId);
      this.sessions.set(repoId, s);
    }
    return s;
  }

  show(repoId: number) {
    this.session(repoId);
    this.shown.add(repoId);
    this.focusTick++;
  }

  hide(repoId: number) {
    this.shown.delete(repoId);
  }

  toggle(repoId: number) {
    if (this.shown.has(repoId)) this.hide(repoId);
    else this.show(repoId);
  }

  /** Ends the shell and goes back to the repo's tabs. */
  close(repoId: number) {
    this.shown.delete(repoId);
    this.sessions.get(repoId)?.dispose();
    this.sessions.delete(repoId);
  }

  restart(repoId: number) {
    const s = this.sessions.get(repoId);
    if (!s) return;
    if (s.id != null) {
      this.byPty.delete(s.id);
      api.terminalClose(s.id).catch(() => {});
      s.id = null;
    }
    s.term.reset();
    s.start();
    this.focusTick++;
  }

  clear(repoId: number) {
    this.sessions.get(repoId)?.term.clear();
  }

  onExit(ptyId: number, code: number) {
    const s = this.byPty.get(ptyId);
    if (!s) return;
    this.byPty.delete(ptyId);
    s.id = null;
    s.exited = code;
    s.term.write(`\r\n\x1b[2m[Shell exited with code ${code}. Press Enter to start a new shell.]\x1b[0m\r\n`);
  }

  applyTheme() {
    const t = themeFromCss();
    for (const s of this.sessions.values()) s.term.options.theme = t;
  }
}

export const terms = new Terms();

if (typeof window !== "undefined") {
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => terms.applyTheme());
}
