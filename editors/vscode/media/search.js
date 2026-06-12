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
    const collapseAllBtn = $('#collapse-all');
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
            resetResults();
            searching = false;
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
            // With a result row selected (↑/↓), Enter opens it instead of
            // re-running the search — see the navigation handler below.
            if (selIdx >= 0) { return; }
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

    // ── Collapse / expand all file groups ────────────────────────────────
    let allCollapsed = false;

    function updateCollapseBtn() {
        collapseAllBtn.textContent = allCollapsed ? '⊞' : '⊟';
        collapseAllBtn.title = allCollapsed ? 'Expand All' : 'Collapse All';
    }

    collapseAllBtn.addEventListener('click', () => {
        allCollapsed = !allCollapsed;
        for (const g of groups.values()) {
            g.matches.hidden = allCollapsed;
            g.twistie.textContent = allCollapsed ? '▸' : '▾';
        }
        updateCollapseBtn();
    });

    // ── Results rendering (grouped by file, like the built-in Search) ────
    // Hits stream in as `batch` messages and accumulate in `allHits`; only
    // `pageSize` rows are added to the DOM per page, with a "Show more"
    // button to append the next page. A file's hits may span batch and page
    // boundaries, so groups are kept in a map and merged into.
    let pageSize = 500;
    let allHits = [];
    let renderedCount = 0;
    let pagesShown = 1;
    let relSet = new Set();
    let searching = false;
    let lastStats = null;
    let groups = new Map(); // rel → {matches, twistie, badge, count}
    let navRows = []; // flat list of match rows, in render order
    let selIdx = -1;
    const showMoreBtn = document.createElement('button');
    showMoreBtn.className = 'show-more';
    showMoreBtn.addEventListener('click', () => {
        pagesShown++;
        renderUpToLimit();
    });

    const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;

    function updateStatus() {
        const total = allHits.length;
        if (searching) {
            statusEl.textContent = total === 0
                ? 'Searching…'
                : `Searching… ${plural(total, 'result')} so far`;
            return;
        }
        if (total === 0) {
            statusEl.textContent =
                `No results found.${lastStats ? ` · ${lastStats.elapsedMs} ms` : ''}`;
            return;
        }
        let summary = `${plural(total, 'result')} in ${plural(relSet.size, 'file')}`;
        if (renderedCount < total) {
            summary += ` — showing first ${renderedCount}`;
        }
        if (lastStats) {
            summary += ` · ${lastStats.elapsedMs} ms`;
            if (lastStats.engine === 'daemon') {
                const hitsN = lastStats.cacheHits || 0;
                summary += hitsN > 0
                    ? ` · ⚡ cached (${hitsN}/${lastStats.filesSearched} files)`
                    : ' · not cached';
            } else {
                summary += ' · not cached (cold run)';
            }
        }
        statusEl.textContent = summary;
    }

    function groupFor(rel) {
        let g = groups.get(rel);
        if (g) return g;

        const slash = Math.max(rel.lastIndexOf('/'), rel.lastIndexOf('\\'));

        const header = document.createElement('div');
        header.className = 'file-row';
        const twistie = document.createElement('span');
        twistie.className = 'twistie';
        twistie.textContent = allCollapsed ? '▸' : '▾';
        const fname = document.createElement('span');
        fname.className = 'fname';
        fname.textContent = slash >= 0 ? rel.slice(slash + 1) : rel;
        const fdir = document.createElement('span');
        fdir.className = 'fdir';
        fdir.textContent = slash >= 0 ? rel.slice(0, slash) : '';
        const badge = document.createElement('span');
        badge.className = 'badge';
        header.append(twistie, fname, fdir, badge);

        const matches = document.createElement('div');
        matches.hidden = allCollapsed;
        header.addEventListener('click', () => {
            matches.hidden = !matches.hidden;
            twistie.textContent = matches.hidden ? '▸' : '▾';
        });

        resultsEl.append(header, matches);
        g = { matches, twistie, badge, count: 0 };
        groups.set(rel, g);
        return g;
    }

    // Fill the .text span with the snippet, wrapping the matched range
    // (byte offsets from the search engine; fine for the ASCII-dominated
    // C++ this searches) in a highlight span.
    function fillSnippet(textEl, h) {
        const s = h.hl_start, e = h.hl_end;
        if (typeof s === 'number' && typeof e === 'number' && s >= 0 && s < e && e <= h.text.length) {
            textEl.append(h.text.slice(0, s));
            const hl = document.createElement('span');
            hl.className = 'hl';
            hl.textContent = h.text.slice(s, e);
            textEl.append(hl, h.text.slice(e));
        } else {
            textEl.textContent = h.text;
        }
    }

    // Render any not-yet-rendered hits, up to the current page limit.
    function renderUpToLimit() {
        const limit = Math.min(allHits.length, pagesShown * pageSize);
        for (let i = renderedCount; i < limit; i++) {
            const h = allHits[i];
            const g = groupFor(h.rel);
            const row = document.createElement('div');
            row.className = 'match-row';
            row.title = `${h.rel}:${h.line}:${h.col} (${h.kind})`;
            const lineno = document.createElement('span');
            lineno.className = 'lineno';
            lineno.textContent = String(h.line);
            const text = document.createElement('span');
            text.className = 'text';
            fillSnippet(text, h);
            row.append(lineno, text);
            const openMsg = {
                type: 'open',
                file: h.file,
                line: h.line,
                col: h.col,
                end_line: h.end_line,
                end_col: h.end_col,
            };
            row.addEventListener('click', () => {
                selectRow(navRows.indexOf(row), false);
                vscode.postMessage(openMsg);
            });
            row.addEventListener('dblclick', () =>
                vscode.postMessage({ ...openMsg, pin: true }),
            );
            row._openMsg = openMsg;
            navRows.push(row);
            g.matches.appendChild(row);
            g.count++;
            g.badge.textContent = String(g.count);
        }
        renderedCount = limit;

        const remaining = allHits.length - renderedCount;
        if (remaining > 0) {
            showMoreBtn.textContent =
                `Show ${Math.min(pageSize, remaining)} more (${remaining} remaining)`;
            resultsEl.appendChild(showMoreBtn); // keep it as the last child
        } else {
            showMoreBtn.remove();
        }
        collapseAllBtn.hidden = renderedCount === 0;
        updateStatus();
    }

    function resetResults() {
        resultsEl.textContent = '';
        groups = new Map();
        navRows = [];
        selIdx = -1;
        allCollapsed = false;
        updateCollapseBtn();
        allHits = [];
        relSet = new Set();
        renderedCount = 0;
        pagesShown = 1;
        lastStats = null;
        collapseAllBtn.hidden = true;
    }

    function appendHits(hits) {
        for (const h of hits) {
            allHits.push(h);
            relSet.add(h.rel);
        }
        renderUpToLimit();
    }

    // ── Keyboard navigation (↑/↓ move, Enter opens, Esc back to input) ───
    function selectRow(idx, reveal) {
        if (idx < 0 || idx >= navRows.length) { return; }
        if (selIdx >= 0 && navRows[selIdx]) {
            navRows[selIdx].classList.remove('selected');
        }
        selIdx = idx;
        const row = navRows[selIdx];
        row.classList.add('selected');
        if (reveal) {
            row.scrollIntoView({ block: 'nearest' });
        }
    }

    // Step over rows hidden inside collapsed file groups. Arrowing past the
    // bottom loads the next page first.
    function moveSelection(delta) {
        let i = selIdx;
        do {
            i += delta;
        } while (i >= 0 && i < navRows.length && navRows[i].parentElement.hidden);
        if (i >= 0 && i < navRows.length) {
            selectRow(i, true);
        } else if (delta > 0 && allHits.length > renderedCount) {
            pagesShown++;
            renderUpToLimit();
            moveSelection(delta);
        } else if (delta < 0) {
            // Moved above the first row — deselect, back to plain typing.
            if (selIdx >= 0 && navRows[selIdx]) {
                navRows[selIdx].classList.remove('selected');
            }
            selIdx = -1;
        }
    }

    function clearSelection() {
        if (selIdx >= 0 && navRows[selIdx]) {
            navRows[selIdx].classList.remove('selected');
        }
        selIdx = -1;
    }

    // Navigation happens from the query input (QuickPick-style): ↑/↓ move
    // the selection, Enter opens it (Ctrl/Cmd+Enter pins), Esc deselects.
    // Enter with no selection re-runs the search (handled by the existing
    // Enter handler above).
    queryEl.addEventListener('keydown', (e) => {
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            moveSelection(1);
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            if (selIdx >= 0) { moveSelection(-1); }
        } else if (e.key === 'Enter' && selIdx >= 0) {
            e.preventDefault();
            const row = navRows[selIdx];
            vscode.postMessage(e.ctrlKey || e.metaKey ? { ...row._openMsg, pin: true } : row._openMsg);
        } else if (e.key === 'Escape' && selIdx >= 0) {
            e.preventDefault();
            clearSelection();
        }
    });

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
            case 'begin':
                if (msg.id !== searchId) return; // stale
                resetResults();
                searching = true;
                updateStatus();
                break;
            case 'batch':
                if (msg.id !== searchId) return; // stale
                appendHits(msg.hits);
                break;
            case 'done':
                if (msg.id !== searchId) return; // stale
                searching = false;
                lastStats = msg.stats || null;
                updateStatus();
                break;
            case 'error':
                if (msg.id !== undefined && msg.id !== searchId) return;
                searching = false;
                resetResults();
                statusEl.textContent = msg.message;
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
                pageSize = Math.max(1, msg.pageSize || 500);
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
