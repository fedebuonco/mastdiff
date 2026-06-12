// mastdiff: Semantic C++ Search — VS Code front-end.
//
// The extension spawns `mastdiff --daemon <root>` on first search. The daemon
// keeps a warm AST cache across queries, so subsequent searches are much
// faster. Each query opens a short-lived TCP connection to the daemon.
//
// Fallback: if the daemon fails to start (binary missing, etc.) the extension
// falls back to the original subprocess-per-search approach.

import * as cp from 'child_process';
import * as fs from 'fs';
import * as net from 'net';
import * as os from 'os';
import * as path from 'path';
import * as readline from 'readline';
import * as vscode from 'vscode';

interface Hit {
    file: string;
    /** 1-based */
    line: number;
    /** 1-based */
    col: number;
    /** 1-based end of the match, for exact-range highlighting */
    end_line?: number;
    end_col?: number;
    text: string;
    kind: string;
    capture: string;
}

/** How a finished search was served — drives the status-line stats. */
interface SearchStats {
    elapsedMs: number;
    engine: 'daemon' | 'subprocess';
    /** Files served from the warm AST cache (daemon only). */
    cacheHits?: number;
    /** Total files searched (daemon only). */
    filesSearched?: number;
}

/** A Hit plus the workspace-relative path the webview displays. */
interface WebviewHit extends Hit {
    rel: string;
}

interface SearchRequest {
    id: number;
    query: string;
    include: string;
    exclude: string;
    caseSensitive: boolean;
    regex: boolean;
}

interface HistoryItem {
    query: string;
    include: string;
    exclude: string;
    caseSensitive: boolean;
    regex: boolean;
    count: number;
    ts: number;
}

const HISTORY_KEY = 'mastdiff.history';

// ── Daemon state ─────────────────────────────────────────────────────────

let daemonPort: number | undefined;
let daemonProc: cp.ChildProcess | undefined;
let daemonStarting: Promise<number | undefined> | undefined;

function config(): vscode.WorkspaceConfiguration {
    return vscode.workspace.getConfiguration('mastdiff');
}

function workspaceRoot(): string | undefined {
    return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

// FNV-1a 64-bit over UTF-8 bytes — must match the Rust implementation in
// src/daemon.rs so that both sides derive the same port file path.
function fnv1a64(s: string): bigint {
    const bytes = Buffer.from(s, 'utf8');
    let hash = 14695981039346656037n;
    const prime = 1099511628211n;
    for (const byte of bytes) {
        hash ^= BigInt(byte);
        hash = BigInt.asUintN(64, hash * prime);
    }
    return hash;
}

function daemonPortFile(root: string): string {
    const canonical = path.resolve(root);
    const hex = fnv1a64(canonical).toString(16).padStart(16, '0');
    return path.join(os.tmpdir(), `mastdiff-${hex}.port`);
}

function tryReadPortFile(root: string): number | undefined {
    try {
        const s = fs.readFileSync(daemonPortFile(root), 'utf8');
        const p = parseInt(s.trim(), 10);
        return isNaN(p) ? undefined : p;
    } catch {
        return undefined;
    }
}

function pingDaemon(port: number): Promise<boolean> {
    return new Promise(resolve => {
        const sock = net.connect(port, '127.0.0.1');
        sock.setTimeout(300);
        sock.on('connect', () => { sock.destroy(); resolve(true); });
        sock.on('error', () => resolve(false));
        sock.on('timeout', () => { sock.destroy(); resolve(false); });
    });
}

function spawnDaemon(root: string): Promise<number | undefined> {
    const bin = config().get<string>('binaryPath', 'mastdiff');
    return new Promise(resolve => {
        const proc = cp.spawn(bin, ['--daemon', root], {
            cwd: root,
            stdio: ['ignore', 'ignore', 'pipe'],
            detached: false,
        });
        daemonProc = proc;

        const rl = readline.createInterface({ input: proc.stderr! });
        rl.on('line', (line: string) => {
            const m = line.match(/^READY (\d+)$/);
            if (m) {
                const port = parseInt(m[1], 10);
                daemonPort = port;
                rl.close();
                proc.stderr?.resume(); // drain remaining stderr to avoid pipe stall
                resolve(port);
            }
        });

        proc.on('error', () => { rl.close(); resolve(undefined); });
        // An immediate exit (e.g. an old binary that doesn't know --daemon)
        // must fail over to the subprocess path right away, not after the
        // READY timeout below.
        proc.on('exit', () => {
            if (daemonProc === proc) {
                daemonProc = undefined;
                daemonPort = undefined;
            }
            rl.close();
            resolve(undefined);
        });

        // Give the daemon up to 20 s to load the project and signal ready.
        setTimeout(() => { rl.close(); resolve(undefined); }, 20_000);
    });
}

async function ensureDaemon(root: string): Promise<number | undefined> {
    // Cached port from this session
    if (daemonPort !== undefined) {
        if (await pingDaemon(daemonPort)) {
            return daemonPort;
        }
        daemonPort = undefined;
    }

    // Another call is already spawning the daemon — share the promise
    if (daemonStarting) {
        return daemonStarting;
    }

    // Daemon may already be running from a previous VS Code session
    const existing = tryReadPortFile(root);
    if (existing !== undefined && await pingDaemon(existing)) {
        daemonPort = existing;
        return existing;
    }

    daemonStarting = spawnDaemon(root).finally(() => { daemonStarting = undefined; });
    return daemonStarting;
}

// ── TCP search ────────────────────────────────────────────────────────────

interface DaemonResult {
    hits: Hit[];
    cacheHits: number;
    filesSearched: number;
}

function runSearchDaemon(port: number, req: SearchRequest): Promise<DaemonResult> {
    return new Promise((resolve, reject) => {
        const payload = JSON.stringify({
            query: req.query,
            include: req.include,
            exclude: req.exclude,
            regex: req.regex,
            case_sensitive: req.caseSensitive,
        }) + '\n';

        const hits: Hit[] = [];
        let cacheHits = 0;
        let filesSearched = 0;
        let settled = false;
        const sock = net.connect(port, '127.0.0.1');
        sock.setTimeout(120_000);

        function done(err?: Error): void {
            if (settled) { return; }
            settled = true;
            sock.destroy();
            if (err) { reject(err); } else { resolve({ hits, cacheHits, filesSearched }); }
        }

        sock.on('connect', () => sock.write(payload));

        let buf = '';
        sock.on('data', (chunk: Buffer) => {
            buf += chunk.toString('utf8');
            let nl: number;
            while ((nl = buf.indexOf('\n')) !== -1) {
                const line = buf.slice(0, nl).trim();
                buf = buf.slice(nl + 1);
                if (!line) { continue; }
                try {
                    const msg = JSON.parse(line) as { type: string; [k: string]: unknown };
                    if (msg.type === 'hit') {
                        hits.push({
                            file: msg.file as string,
                            line: msg.line as number,
                            col: msg.col as number,
                            end_line: msg.end_line as number,
                            end_col: msg.end_col as number,
                            text: msg.text as string,
                            kind: msg.kind as string,
                            capture: msg.capture as string,
                        });
                    } else if (msg.type === 'done') {
                        cacheHits = (msg.cache_hits as number) ?? 0;
                        filesSearched = (msg.files as number) ?? 0;
                        done();
                    } else if (msg.type === 'error') {
                        done(new Error(msg.message as string));
                    }
                } catch { /* ignore malformed lines */ }
            }
        });

        sock.on('close', () => done());
        sock.on('error', (e) => done(e));
        sock.on('timeout', () => done(new Error('daemon search timed out')));
    });
}

// ── Subprocess fallback ───────────────────────────────────────────────────

let activeProc: cp.ChildProcess | undefined;

function runSearchSubprocess(req: SearchRequest, root: string): Promise<Hit[]> {
    if (activeProc) {
        activeProc.kill();
        activeProc = undefined;
    }
    return new Promise((resolve, reject) => {
        const bin = config().get<string>('binaryPath', 'mastdiff');
        const args = ['--search', req.query, '--json'];
        if (req.include.trim()) { args.push('--include', req.include.trim()); }
        if (req.exclude.trim()) { args.push('--exclude', req.exclude.trim()); }
        if (req.regex) { args.push('--regex'); }
        if (req.caseSensitive) { args.push('--case-sensitive'); }
        args.push(root);

        const proc = cp.spawn(bin, args, { cwd: root });
        activeProc = proc;

        const hits: Hit[] = [];
        let stderr = '';
        const rl = readline.createInterface({ input: proc.stdout });
        rl.on('line', (line) => {
            try { hits.push(JSON.parse(line) as Hit); } catch { /* ignore */ }
        });
        proc.stderr.on('data', (chunk) => (stderr += chunk));
        proc.on('error', (err) => reject(err));
        proc.on('close', (code, signal) => {
            if (activeProc === proc) { activeProc = undefined; }
            if (signal) {
                resolve([]);
            } else if (code !== 0) {
                reject(new Error(stderr.trim() || `mastdiff exited with code ${code}`));
            } else {
                resolve(hits);
            }
        });
    });
}

function friendlyError(err: unknown): string {
    const msg = err instanceof Error ? err.message : String(err);
    if (msg.includes('ENOENT')) {
        const bin = config().get<string>('binaryPath', 'mastdiff');
        return (
            `mastdiff binary not found ('${bin}'). Install mastdiff or set "mastdiff.binaryPath" ` +
            `(https://github.com/fedebuonco/mastdiff/releases).`
        );
    }
    return msg;
}

// ── Open file ─────────────────────────────────────────────────────────────

// Same highlight style the built-in Search uses when revealing a match.
const matchDecoration = vscode.window.createTextEditorDecorationType({
    backgroundColor: new vscode.ThemeColor('editor.findMatchHighlightBackground'),
    borderRadius: '2px',
});

async function revealHit(
    hit: { file: string; line: number; col: number; end_line?: number; end_col?: number },
    preview: boolean,
): Promise<void> {
    const start = new vscode.Position(hit.line - 1, hit.col - 1);
    const end = hit.end_line && hit.end_col
        ? new vscode.Position(hit.end_line - 1, hit.end_col - 1)
        : start;
    const range = new vscode.Range(start, end);
    const doc = await vscode.workspace.openTextDocument(hit.file);
    const editor = await vscode.window.showTextDocument(doc, {
        preview,
        preserveFocus: preview,
        selection: range,
    });
    // Flash-highlight the exact match range, then fade it out.
    editor.setDecorations(matchDecoration, [range]);
    setTimeout(() => editor.setDecorations(matchDecoration, []), 2500);
}

// ── Search view ───────────────────────────────────────────────────────────

class SearchViewProvider implements vscode.WebviewViewProvider {
    public static readonly viewType = 'mastdiff.searchView';

    private view?: vscode.WebviewView;
    private pendingQuery?: string;
    private generation = 0;

    constructor(private readonly ctx: vscode.ExtensionContext) {}

    resolveWebviewView(view: vscode.WebviewView): void {
        this.view = view;
        view.webview.options = {
            enableScripts: true,
            localResourceRoots: [vscode.Uri.joinPath(this.ctx.extensionUri, 'media')],
        };
        view.webview.html = this.renderHtml(view.webview);
        view.webview.onDidReceiveMessage((msg) => this.onMessage(msg));
        view.onDidDispose(() => {
            if (this.view === view) { this.view = undefined; }
        });
    }

    focus(): void {
        vscode.commands.executeCommand(`${SearchViewProvider.viewType}.focus`);
    }

    setQueryAndRun(query: string): void {
        if (this.view) {
            this.view.show?.(true);
            this.post({ type: 'setQuery', value: query, run: true });
        } else {
            this.pendingQuery = query;
            this.focus();
        }
    }

    private post(msg: unknown): void {
        this.view?.webview.postMessage(msg);
    }

    private history(): HistoryItem[] {
        return this.ctx.workspaceState.get<HistoryItem[]>(HISTORY_KEY, []);
    }

    private addHistory(req: SearchRequest, count: number): void {
        const same = (h: HistoryItem) =>
            h.query === req.query && h.include === req.include &&
            h.exclude === req.exclude && h.caseSensitive === req.caseSensitive &&
            h.regex === req.regex;
        const max = Math.max(1, config().get<number>('historySize', 50));
        const entry: HistoryItem = {
            query: req.query, include: req.include, exclude: req.exclude,
            caseSensitive: req.caseSensitive, regex: req.regex, count, ts: Date.now(),
        };
        const items = [entry, ...this.history().filter((h) => !same(h))].slice(0, max);
        this.ctx.workspaceState.update(HISTORY_KEY, items);
    }

    private async onMessage(msg: any): Promise<void> {
        switch (msg.type) {
            case 'ready':
                this.post({
                    type: 'init',
                    history: this.history(),
                    include: config().get<string>('include', ''),
                    exclude: config().get<string>('exclude', ''),
                    debounceMs: config().get<number>('debounceMs', 300),
                    pageSize: config().get<number>('maxResults', 500),
                    query: this.pendingQuery ?? '',
                });
                this.pendingQuery = undefined;
                break;
            case 'search':
                await this.doSearch(msg as SearchRequest);
                break;
            case 'open':
                revealHit(msg, msg.pin !== true).then(undefined, () => undefined);
                break;
            case 'clearHistory':
                await this.ctx.workspaceState.update(HISTORY_KEY, []);
                this.post({ type: 'history', items: [] });
                break;
        }
    }

    private async doSearch(req: SearchRequest): Promise<void> {
        const root = workspaceRoot();
        if (!root) {
            this.post({ type: 'error', id: req.id, message: 'Open a folder to search.' });
            return;
        }

        const mine = ++this.generation;

        try {
            const t0 = Date.now();
            let hits: Hit[];
            let stats: SearchStats;

            // Try daemon first; fall back to subprocess on any failure.
            const port = await ensureDaemon(root);
            if (port !== undefined) {
                try {
                    const res = await runSearchDaemon(port, req);
                    hits = res.hits;
                    stats = {
                        elapsedMs: Date.now() - t0,
                        engine: 'daemon',
                        cacheHits: res.cacheHits,
                        filesSearched: res.filesSearched,
                    };
                } catch (daemonErr) {
                    // Daemon died or returned an error — clear port and fall back.
                    daemonPort = undefined;
                    hits = await runSearchSubprocess(req, root);
                    stats = { elapsedMs: Date.now() - t0, engine: 'subprocess' };
                }
            } else {
                hits = await runSearchSubprocess(req, root);
                stats = { elapsedMs: Date.now() - t0, engine: 'subprocess' };
            }

            if (mine !== this.generation) { return; }

            const shown: WebviewHit[] = hits.map((h) => {
                const abs = path.isAbsolute(h.file) ? h.file : path.join(root, h.file);
                return { ...h, file: abs, rel: path.relative(root, abs) };
            });
            this.post({ type: 'results', id: req.id, hits: shown, total: hits.length, stats });
            this.addHistory(req, hits.length);
            this.post({ type: 'history', items: this.history() });
        } catch (err) {
            if (mine !== this.generation) { return; }
            this.post({ type: 'error', id: req.id, message: friendlyError(err) });
        }
    }

    private renderHtml(webview: vscode.Webview): string {
        const media = (file: string) =>
            webview.asWebviewUri(vscode.Uri.joinPath(this.ctx.extensionUri, 'media', file));
        const nonce = Array.from({ length: 32 }, () =>
            'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'.charAt(Math.floor(Math.random() * 62)),
        ).join('');
        return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link href="${media('search.css')}" rel="stylesheet">
<title>mastdiff search</title>
</head>
<body>
  <div class="tabs" role="tablist">
    <button id="tab-search" class="tab active" role="tab">Search</button>
    <button id="tab-history" class="tab" role="tab">History</button>
  </div>

  <div id="pane-search" role="tabpanel">
    <div class="input-box">
      <input id="query" type="text" spellcheck="false"
             placeholder="Search (fn:update, call:render, class:Entity, plain text…)">
      <div class="input-actions">
        <button id="toggle-case" title="Match Case">Aa</button>
        <button id="toggle-regex" title="Use Regular Expression">.*</button>
      </div>
    </div>
    <div id="chips" title="Semantic filters — click to apply a tree-sitter query prefix"></div>
    <details id="globs">
      <summary>files to include / exclude</summary>
      <input id="include" type="text" spellcheck="false" placeholder="files to include (e.g. src/**,*.hpp)">
      <input id="exclude" type="text" spellcheck="false" placeholder="files to exclude (e.g. tests/**,vendor/**)">
    </details>
    <div id="results-bar">
      <span id="status"></span>
      <button id="collapse-all" hidden title="Collapse All">⊟</button>
    </div>
    <div id="results"></div>
  </div>

  <div id="pane-history" role="tabpanel" hidden>
    <div class="history-header">
      <span>Previous searches</span>
      <button id="clear-history" title="Clear search history">Clear</button>
    </div>
    <ul id="history-list"></ul>
  </div>

  <script nonce="${nonce}" src="${media('search.js')}"></script>
</body>
</html>`;
    }
}

// ── Word under cursor ──────────────────────────────────────────────────────

function wordUnderCursor(): string | undefined {
    const editor = vscode.window.activeTextEditor;
    if (!editor) { return undefined; }
    const sel = editor.selection;
    if (!sel.isEmpty) { return editor.document.getText(sel); }
    const range = editor.document.getWordRangeAtPosition(sel.active);
    return range ? editor.document.getText(range) : undefined;
}

// ── Activation / deactivation ─────────────────────────────────────────────

export function activate(context: vscode.ExtensionContext): void {
    const provider = new SearchViewProvider(context);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider(SearchViewProvider.viewType, provider),
        vscode.commands.registerCommand('mastdiff.search', () => provider.focus()),
        vscode.commands.registerCommand('mastdiff.findCalls', () => {
            const word = wordUnderCursor();
            provider.setQueryAndRun(word ? `call:${word}` : 'call:');
        }),
        vscode.commands.registerCommand('mastdiff.findDefinitions', () => {
            const word = wordUnderCursor();
            provider.setQueryAndRun(word ? `fn:${word}` : 'fn:');
        }),
    );

    // Kick off daemon pre-warm as soon as a workspace is open, so the first
    // search is instant rather than having to wait for project discovery.
    const root = workspaceRoot();
    if (root) {
        ensureDaemon(root).then(
            (port) => { if (port) { /* daemon ready */ } },
            () => { /* silently ignore — subprocess fallback covers it */ },
        );
    }
}

export function deactivate(): void {
    if (activeProc) {
        activeProc.kill();
        activeProc = undefined;
    }
    if (daemonProc) {
        daemonProc.kill();
        daemonProc = undefined;
    }
    matchDecoration.dispose();
}
