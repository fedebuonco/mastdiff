// mastdiff: Semantic C++ Search — a VS Code front-end for the mastdiff CLI.
//
// The extension is a thin client: every query is executed by
// `mastdiff --search <query> --json <root>`, which runs native tree-sitter
// queries over the whole project in parallel and prints one JSON object per
// result line. The extension streams those into a live QuickPick.

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

interface HitItem extends vscode.QuickPickItem {
    hit: Hit;
}

let activeProc: cp.ChildProcess | undefined;

function config(): vscode.WorkspaceConfiguration {
    return vscode.workspace.getConfiguration('mastdiff');
}

function workspaceRoot(): string | undefined {
    return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

/** Run one headless mastdiff search. Kills any search still in flight. */
function runSearch(query: string, root: string): Promise<Hit[]> {
    if (activeProc) {
        activeProc.kill();
        activeProc = undefined;
    }
    return new Promise((resolve, reject) => {
        const bin = config().get<string>('binaryPath', 'mastdiff');
        const args = ['--search', query, '--json'];
        const include = config().get<string>('include', '').trim();
        const exclude = config().get<string>('exclude', '').trim();
        if (include) {
            args.push('--include', include);
        }
        if (exclude) {
            args.push('--exclude', exclude);
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
                // Superseded by a newer search — resolve empty, caller's
                // generation counter discards this anyway.
                resolve([]);
            } else if (code !== 0) {
                reject(new Error(stderr.trim() || `mastdiff exited with code ${code}`));
            } else {
                resolve(hits);
            }
        });
    });
}

/** Pick a codicon for the result based on what tree-sitter captured. */
function iconFor(hit: Hit): string {
    switch (hit.kind) {
        case 'type_identifier':
            return '$(symbol-class)';
        case 'field_identifier':
            return '$(symbol-field)';
        case 'identifier':
            return '$(symbol-method)';
        case 'lambda_expression':
            return '$(symbol-function)';
        case 'string_literal':
        case 'system_lib_string':
            return '$(file-symlink-file)'; // include: results
        case 'text':
            return '$(search)';
        default:
            return '$(symbol-misc)';
    }
}

function toItem(root: string, hit: Hit): HitItem {
    const abs = path.isAbsolute(hit.file) ? hit.file : path.join(root, hit.file);
    const rel = path.relative(root, abs);
    return {
        // alwaysShow bypasses the QuickPick's own fuzzy filter: the value the
        // user types is a mastdiff query, not a substring of the results.
        alwaysShow: true,
        label: `${iconFor(hit)} ${hit.text}`,
        description: `${rel}:${hit.line}`,
        hit: { ...hit, file: abs },
    };
}

async function revealHit(hit: Hit, preview: boolean): Promise<void> {
    const pos = new vscode.Position(hit.line - 1, hit.col - 1);
    const doc = await vscode.workspace.openTextDocument(hit.file);
    await vscode.window.showTextDocument(doc, {
        preview,
        preserveFocus: preview,
        selection: new vscode.Range(pos, pos),
    });
}

function showSearchError(err: unknown): void {
    const msg = err instanceof Error ? err.message : String(err);
    if (msg.includes('ENOENT')) {
        const bin = config().get<string>('binaryPath', 'mastdiff');
        vscode.window.showErrorMessage(
            `mastdiff binary not found ('${bin}'). Install mastdiff or set "mastdiff.binaryPath" ` +
                `(https://github.com/fedebuonco/mastdiff/releases).`,
        );
    } else {
        vscode.window.showErrorMessage(`mastdiff search failed: ${msg}`);
    }
}

const PLACEHOLDER =
    'fn:update · call:render · class:Entity · var:player · field:velocity · include:audio · plain text · (raw s-expr) @cap';

function openSearch(initialQuery?: string): void {
    const root = workspaceRoot();
    if (!root) {
        vscode.window.showErrorMessage('mastdiff: open a folder to search.');
        return;
    }

    const qp = vscode.window.createQuickPick<HitItem>();
    qp.title = 'mastdiff — semantic C++ search';
    qp.placeholder = PLACEHOLDER;
    qp.matchOnDescription = false;

    // Generation counter: only the latest search may touch the UI.
    let generation = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const fire = (value: string) => {
        const mine = ++generation;
        if (!value.trim()) {
            qp.items = [];
            qp.busy = false;
            return;
        }
        qp.busy = true;
        runSearch(value, root)
            .then((hits) => {
                if (mine !== generation) {
                    return;
                }
                const max = config().get<number>('maxResults', 500);
                qp.items = hits.slice(0, max).map((h) => toItem(root, h));
                qp.busy = false;
            })
            .catch((err) => {
                if (mine !== generation) {
                    return;
                }
                qp.items = [];
                qp.busy = false;
                showSearchError(err);
            });
    };

    const debounced = (value: string) => {
        if (timer) {
            clearTimeout(timer);
        }
        timer = setTimeout(() => fire(value), config().get<number>('debounceMs', 300));
    };

    qp.onDidChangeValue(debounced);

    qp.onDidChangeActive((items) => {
        if (config().get<boolean>('previewResults', true) && items[0]) {
            revealHit(items[0].hit, true).then(undefined, () => undefined);
        }
    });

    qp.onDidAccept(() => {
        const item = qp.selectedItems[0];
        if (!item) {
            return;
        }
        qp.hide();
        revealHit(item.hit, false).then(undefined, () => undefined);
    });

    qp.onDidHide(() => {
        generation++;
        if (timer) {
            clearTimeout(timer);
        }
        if (activeProc) {
            activeProc.kill();
            activeProc = undefined;
        }
        qp.dispose();
    });

    if (initialQuery) {
        qp.value = initialQuery;
        fire(initialQuery);
    }
    qp.show();
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
    context.subscriptions.push(
        vscode.commands.registerCommand('mastdiff.search', () => openSearch()),
        vscode.commands.registerCommand('mastdiff.findCalls', () => {
            const word = wordUnderCursor();
            openSearch(word ? `call:${word}` : 'call:');
        }),
        vscode.commands.registerCommand('mastdiff.findDefinitions', () => {
            const word = wordUnderCursor();
            openSearch(word ? `fn:${word}` : 'fn:');
        }),
    );
}

export function deactivate(): void {
    if (activeProc) {
        activeProc.kill();
        activeProc = undefined;
    }
}
