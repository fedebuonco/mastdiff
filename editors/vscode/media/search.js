// Webview script for the mastdiff search view.
// Talks to the extension host via postMessage; the host runs the actual
// `mastdiff --search --json` process and sends results back.
(function () {
    'use strict';
    const vscode = acquireVsCodeApi();

    const $ = (sel) => document.querySelector(sel);
    const queryEl = $('#query');
    const includeEl = $('#include');
    const excludeEl = $('#exclude');
    const caseBtn = $('#toggle-case');
    const regexBtn = $('#toggle-regex');
    const chipsEl = $('#chips');
    const globsEl = $('#globs');
    const statusEl = $('#status');
    const resultsEl = $('#results');
    const historyList = $('#history-list');
    const tabSearch = $('#tab-search');
    const tabHistory = $('#tab-history');
    const paneSearch = $('#pane-search');
    const paneHistory = $('#pane-history');

    let caseSensitive = false;
    let useRegex = false;
    let searchId = 0;
    let debounceTimer;
    let debounceMs = 300;

    // ── Filter chips: one colored block per semantic query prefix ────────
    const CHIPS = [
        ['fn:', '#569cd6', 'function definitions'],
        ['call:', '#dcdcaa', 'call sites (direct calls only)'],
        ['class:', '#4ec9b0', 'class / struct / enum declarations'],
        ['var:', '#9cdcfe', 'variable declarations'],
        ['param:', '#c586c0', 'function parameters'],
        ['field:', '#4fc1ff', 'struct / class fields'],
        ['include:', '#ce9178', '#include directives'],
        ['macro:', '#d16969', '#define macros'],
        ['ns:', '#b5cea8', 'namespace definitions'],
        ['type:', '#4ec9b0', 'type identifiers'],
        ['lambda:', '#dcdcaa', 'lambda expressions'],
        ['tpl:', '#569cd6', 'template functions / classes'],
        ['throw:', '#d16969', 'throw statements'],
        ['cast:', '#ce9178', 'C and C++ casts'],
        ['op:', '#c586c0', 'operator overload definitions'],
        ['using:', '#9cdcfe', 'using declarations'],
    ];
    const PREFIX_RE = /^[a-z_]+:/;

    for (const [prefix, color, desc] of CHIPS) {
        const b = document.createElement('button');
        b.className = 'chip';
        b.textContent = prefix;
        b.title = desc;
        b.style.setProperty('--chip-color', color);
        b.addEventListener('click', () => {
            const m = queryEl.value.match(PREFIX_RE);
            if (m && m[0] === prefix) {
                queryEl.value = queryEl.value.slice(prefix.length); // toggle off
            } else if (m) {
                queryEl.value = prefix + queryEl.value.slice(m[0].length); // swap prefix
            } else {
                queryEl.value = prefix + queryEl.value;
            }
            queryEl.focus();
            updateChips();
            fire();
        });
        chipsEl.appendChild(b);
    }

    function updateChips() {
        for (const el of chipsEl.children) {
            el.classList.toggle('active', queryEl.value.startsWith(el.textContent));
        }
    }

    // ── Search plumbing ───────────────────────────────────────────────────
    function persist() {
        vscode.setState({
            query: queryEl.value,
            include: includeEl.value,
            exclude: excludeEl.value,
            caseSensitive,
            useRegex,
        });
    }

    function fire() {
        persist();
        const query = queryEl.value.trim();
        searchId++;
        if (!query) {
            resultsEl.textContent = '';
            statusEl.textContent = '';
            return;
        }
        statusEl.textContent = 'Searching…';
        vscode.postMessage({
            type: 'search',
            id: searchId,
            query,
            include: includeEl.value.trim(),
            exclude: excludeEl.value.trim(),
            caseSensitive,
            regex: useRegex,
        });
    }

    function debouncedFire() {
        clearTimeout(debounceTimer);
        debounceTimer = setTimeout(fire, debounceMs);
    }

    queryEl.addEventListener('input', () => {
        updateChips();
        debouncedFire();
    });
    queryEl.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
            clearTimeout(debounceTimer);
            fire();
        }
    });
    includeEl.addEventListener('input', debouncedFire);
    excludeEl.addEventListener('input', debouncedFire);

    caseBtn.addEventListener('click', () => {
        caseSensitive = !caseSensitive;
        caseBtn.classList.toggle('on', caseSensitive);
        fire();
    });
    regexBtn.addEventListener('click', () => {
        useRegex = !useRegex;
        regexBtn.classList.toggle('on', useRegex);
        fire();
    });

    // ── Tabs ──────────────────────────────────────────────────────────────
    function showTab(which) {
        tabSearch.classList.toggle('active', which === 'search');
        tabHistory.classList.toggle('active', which === 'history');
        paneSearch.hidden = which !== 'search';
        paneHistory.hidden = which !== 'history';
    }
    tabSearch.addEventListener('click', () => showTab('search'));
    tabHistory.addEventListener('click', () => showTab('history'));
    $('#clear-history').addEventListener('click', () => vscode.postMessage({ type: 'clearHistory' }));

    // ── Results rendering (grouped by file, like the built-in Search) ────
    function renderResults(hits, total) {
        resultsEl.textContent = '';
        const byFile = new Map();
        for (const h of hits) {
            if (!byFile.has(h.rel)) {
                byFile.set(h.rel, []);
            }
            byFile.get(h.rel).push(h);
        }

        const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;
        let summary = `${plural(total, 'result')} in ${plural(byFile.size, 'file')}`;
        if (total > hits.length) {
            summary += ` — showing first ${hits.length}`;
        }
        statusEl.textContent = total === 0 ? 'No results found.' : summary;

        for (const [rel, fileHits] of byFile) {
            const slash = Math.max(rel.lastIndexOf('/'), rel.lastIndexOf('\\'));

            const header = document.createElement('div');
            header.className = 'file-row';
            const twistie = document.createElement('span');
            twistie.className = 'twistie';
            twistie.textContent = '▾';
            const fname = document.createElement('span');
            fname.className = 'fname';
            fname.textContent = slash >= 0 ? rel.slice(slash + 1) : rel;
            const fdir = document.createElement('span');
            fdir.className = 'fdir';
            fdir.textContent = slash >= 0 ? rel.slice(0, slash) : '';
            const badge = document.createElement('span');
            badge.className = 'badge';
            badge.textContent = String(fileHits.length);
            header.append(twistie, fname, fdir, badge);

            const matches = document.createElement('div');
            for (const h of fileHits) {
                const row = document.createElement('div');
                row.className = 'match-row';
                row.title = `${rel}:${h.line}:${h.col} (${h.kind})`;
                const lineno = document.createElement('span');
                lineno.className = 'lineno';
                lineno.textContent = String(h.line);
                const text = document.createElement('span');
                text.className = 'text';
                text.textContent = h.text;
                row.append(lineno, text);
                row.addEventListener('click', () =>
                    vscode.postMessage({ type: 'open', file: h.file, line: h.line, col: h.col }),
                );
                row.addEventListener('dblclick', () =>
                    vscode.postMessage({ type: 'open', file: h.file, line: h.line, col: h.col, pin: true }),
                );
                matches.appendChild(row);
            }

            header.addEventListener('click', () => {
                matches.hidden = !matches.hidden;
                twistie.textContent = matches.hidden ? '▸' : '▾';
            });

            resultsEl.append(header, matches);
        }
    }

    // ── History rendering ─────────────────────────────────────────────────
    function timeAgo(ts) {
        const s = Math.max(1, Math.round((Date.now() - ts) / 1000));
        if (s < 60) return `${s}s ago`;
        const m = Math.round(s / 60);
        if (m < 60) return `${m}m ago`;
        const h = Math.round(m / 60);
        if (h < 24) return `${h}h ago`;
        return `${Math.round(h / 24)}d ago`;
    }

    function renderHistory(items) {
        historyList.textContent = '';
        if (!items.length) {
            const li = document.createElement('li');
            li.className = 'hist-empty';
            li.textContent = 'No previous searches yet.';
            historyList.appendChild(li);
            return;
        }
        for (const it of items) {
            const li = document.createElement('li');
            li.className = 'hist-row';
            const q = document.createElement('span');
            q.className = 'hquery';
            q.textContent = it.query;
            const meta = document.createElement('span');
            meta.className = 'hmeta';
            const parts = [`${it.count} result${it.count === 1 ? '' : 's'}`, timeAgo(it.ts)];
            if (it.include) parts.push(`include: ${it.include}`);
            if (it.exclude) parts.push(`exclude: ${it.exclude}`);
            if (it.caseSensitive) parts.push('Aa');
            if (it.regex) parts.push('.*');
            meta.textContent = parts.join(' · ');
            li.append(q, meta);
            li.addEventListener('click', () => {
                queryEl.value = it.query;
                includeEl.value = it.include || '';
                excludeEl.value = it.exclude || '';
                caseSensitive = !!it.caseSensitive;
                useRegex = !!it.regex;
                caseBtn.classList.toggle('on', caseSensitive);
                regexBtn.classList.toggle('on', useRegex);
                if (includeEl.value || excludeEl.value) {
                    globsEl.open = true;
                }
                showTab('search');
                updateChips();
                fire();
            });
            historyList.appendChild(li);
        }
    }

    // ── Messages from the extension host ──────────────────────────────────
    window.addEventListener('message', (e) => {
        const msg = e.data;
        switch (msg.type) {
            case 'results':
                if (msg.id !== searchId) return; // stale
                renderResults(msg.hits, msg.total);
                break;
            case 'error':
                if (msg.id !== undefined && msg.id !== searchId) return;
                statusEl.textContent = msg.message;
                resultsEl.textContent = '';
                break;
            case 'history':
                renderHistory(msg.items);
                break;
            case 'setQuery':
                showTab('search');
                queryEl.value = msg.value;
                queryEl.focus();
                updateChips();
                if (msg.run) fire();
                break;
            case 'init': {
                debounceMs = msg.debounceMs || 300;
                renderHistory(msg.history);
                const st = vscode.getState();
                if (st) {
                    queryEl.value = st.query || '';
                    includeEl.value = st.include || '';
                    excludeEl.value = st.exclude || '';
                    caseSensitive = !!st.caseSensitive;
                    useRegex = !!st.useRegex;
                    caseBtn.classList.toggle('on', caseSensitive);
                    regexBtn.classList.toggle('on', useRegex);
                } else {
                    includeEl.value = msg.include || '';
                    excludeEl.value = msg.exclude || '';
                }
                if (msg.query) {
                    queryEl.value = msg.query; // command-supplied query wins
                }
                if (includeEl.value || excludeEl.value) {
                    globsEl.open = true;
                }
                updateChips();
                if (queryEl.value.trim()) {
                    fire();
                }
                queryEl.focus();
                break;
            }
        }
    });

    vscode.postMessage({ type: 'ready' });
})();
