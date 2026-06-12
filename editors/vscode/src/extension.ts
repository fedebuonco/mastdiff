// mastdiff: Semantic C++ Search — a VS Code front-end for the mastdiff CLI.
//
// The extension is a thin client: every query is executed by
// `mastdiff --search <query> --json <root>`, which runs native tree-sitter
// queries over the whole project in parallel and prints one JSON object per
// result line. The UI is a sidebar webview styled after the built-in Search
// view: query box with filter chips, results grouped by file underneath,
// and a History tab with previous searches.

import * as cp from 'child_process';
import * as path from 'path';
import * as readline from 'readline';
import * as vscode from 'vscode';

interface Hit {
    file: string;
    /** 1-based */
    line: number;
    /** 1-based */
    col: number;
    text: string;
    kind: string;
    capture: string;
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

let activeProc: cp.ChildProcess | undefined;

function config(): vscode.WorkspaceConfiguration {
    return vscode.workspace.getConfiguration('mastdiff');
}

function workspaceRoot(): string | undefined {
    return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

/** Run one headless mastdiff search. Kills any search still in flight. */
function runSearch(req: SearchRequest, root: string): Promise<Hit[]> {
    if (activeProc) {
        activeProc.kill();
        activeProc = undefined;
    }
    return new Promise((resolve, reject) => {
        const bin = config().get<string>('binaryPath', 'mastdiff');
        const args = ['--search', req.query, '--json'];
        if (req.include.trim()) {
            args.push('--include', req.include.trim());
        }
        if (req.exclude.trim()) {
            args.push('--exclude', req.exclude.trim());
        }
        if (req.regex) {
            args.push('--regex');
        }
        if (req.caseSensitive) {
            args.push('--case-sensitive');
        }
        args.push(root);

        const proc = cp.spawn(bin, args, { cwd: root });
        activeProc = proc;

        const hits: Hit[] = [];
        let stderr = '';
        const rl = readline.createInterface({ input: proc.stdout });
        rl.on('line', (line) => {
            try {
                hits.push(JSON.parse(line) as Hit);
            } catch {
                // ignore non-JSON noise on stdout
            }
        });
        proc.stderr.on('data', (chunk) => (stderr += chunk));
        proc.on('error', (err) => reject(err));
        proc.on('close', (code, signal) => {
            if (activeProc === proc) {
                activeProc = undefined;
            }
            if (signal) {
                // Superseded by a newer search — the caller's generation
                // counter discards this result set anyway.
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

async function revealHit(hit: { file: string; line: number; col: number }, preview: boolean): Promise<void> {
    const pos = new vscode.Position(hit.line - 1, hit.col - 1);
    const doc = await vscode.workspace.openTextDocument(hit.file);
    await vscode.window.showTextDocument(doc, {
        preview,
        preserveFocus: preview,
        selection: new vscode.Range(pos, pos),
    });
}

class SearchViewProvider implements vscode.WebviewViewProvider {
    public static readonly viewType = 'mastdiff.searchView';

    private view?: vscode.WebviewView;
    /** Query queued by a command before the webview finished loading. */
    private pendingQuery?: string;
    /** Only the most recent search may publish results. */
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
            if (this.view === view) {
                this.view = undefined;
            }
        });
    }

    /** Focus the view, creating it if needed. */
    focus(): void {
        vscode.commands.executeCommand(`${SearchViewProvider.viewType}.focus`);
    }

    /** Focus the view and run `query` in it. */
    setQueryAndRun(query: string): void {
        if (this.view) {
            this.view.show?.(true);
            this.post({ type: 'setQuery', value: query, run: true });
        } else {
            this.pendingQuery = query;
            this.focus(); // resolveWebviewView → webview sends 'ready' → init carries pendingQuery
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
            h.query === req.query &&
            h.include === req.include &&
            h.exclude === req.exclude &&
            h.caseSensitive === req.caseSensitive &&
            h.regex === req.regex;
        const max = Math.max(1, config().get<number>('historySize', 50));
        const entry: HistoryItem = {
            query: req.query,
            include: req.include,
            exclude: req.exclude,
            caseSensitive: req.caseSensitive,
            regex: req.regex,
            count,
            ts: Date.now(),
        };
        const items = [entry, ...this.history().filter((h) => !same(h))].slice(0, max);
        this.ctx.workspaceState.update(HISTORY_KEY, items);
    }

    private async onMessage(msg: any): Promise<void> {
        switch (msg.type) {
            case 'ready': {
                this.post({
                    type: 'init',
                    history: this.history(),
                    include: config().get<string>('include', ''),
                    exclude: config().get<string>('exclude', ''),
                    debounceMs: config().get<number>('debounceMs', 300),
                    query: this.pendingQuery ?? '',
                });
                this.pendingQuery = undefined;
                break;
            }
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
            const hits = await runSearch(req, root);
            if (mine !== this.generation) {
                return;
            }
            const max = config().get<number>('maxResults', 500);
            const shown: WebviewHit[] = hits.slice(0, max).map((h) => {
                const abs = path.isAbsolute(h.file) ? h.file : path.join(root, h.file);
                return { ...h, file: abs, rel: path.relative(root, abs) };
            });
            this.post({ type: 'results', id: req.id, hits: shown, total: hits.length });
            this.addHistory(req, hits.length);
            this.post({ type: 'history', items: this.history() });
        } catch (err) {
            if (mine !== this.generation) {
                return;
            }
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
    <div id="status"></div>
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

function wordUnderCursor(): string | undefined {
    const editor = vscode.window.activeTextEditor;
    if (!editor) {
        return undefined;
    }
    const sel = editor.selection;
    if (!sel.isEmpty) {
        return editor.document.getText(sel);
    }
    const range = editor.document.getWordRangeAtPosition(sel.active);
    return range ? editor.document.getText(range) : undefined;
}

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
}

export function deactivate(): void {
    if (activeProc) {
        activeProc.kill();
        activeProc = undefined;
    }
}
