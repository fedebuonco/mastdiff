# Installation and Development

## Install mastdiff binary (required)

The extension needs the `mastdiff` CLI binary. Get it from releases or build it:

### Option 1: Pre-built binary (easiest)
Download from [https://github.com/fedebuonco/mastdiff/releases](https://github.com/fedebuonco/mastdiff/releases):
- `mastdiff-linux-x86_64` (Linux)
- `mastdiff-macos-aarch64` (macOS ARM)
- `mastdiff-macos-x86_64` (macOS Intel)
- `mastdiff-windows-x86_64.exe` (Windows)

Rename it to `mastdiff` (drop the platform suffix) and place on your `PATH`, or set `mastdiff.binaryPath` in VS Code settings to the full path.

### Option 2: Build from source
```bash
git clone https://github.com/fedebuonco/mastdiff
cd mastdiff
cargo build --release
# binary at target/release/mastdiff
# add to PATH or use mastdiff.binaryPath setting
```

## Run the extension in development

```bash
cd editors/vscode
npm install          # one-time: install devDependencies
npm run compile      # compile TypeScript → out/
```

Then in VS Code:
1. **Open this folder** (`File` → `Open Folder` → `mastdiff/editors/vscode`)
2. **Press F5** to launch an "Extension Development Host" (a new VS Code window with your extension loaded)
3. In that window, click the mastdiff icon in the Activity Bar (left sidebar, bottom) or press `Ctrl+Alt+F`
4. Type a query: `fn:`, `call:update`, `class:`, or plain text
5. Click results to open them; double-click to pin the editor

The extension is live-reloadable: edit `media/search.js`, `media/search.css`, or `src/extension.ts`, recompile (`npm run compile`), then press `Ctrl+Shift+F5` in the dev host to reload.

## Package and publish

To distribute it as a `.vsix` file (shareable, installable offline):

```bash
npm install -g @vscode/vsce
vsce package
# produces mastdiff-search-0.2.0.vsix
```

Install locally with:
```
code --install-extension mastdiff-search-0.2.0.vsix
```

To publish to the VS Code Marketplace, you need a publisher account (see [vsce docs](https://github.com/microsoft/vscode-vsce)).

## Settings

Configure in VS Code settings (JSON or UI):

```json
{
  "mastdiff.binaryPath": "mastdiff",           // path to the CLI binary
  "mastdiff.include": "src/**,*.hpp",           // default include globs
  "mastdiff.exclude": "tests/**,vendor/**",     // default exclude globs
  "mastdiff.debounceMs": 300,                   // live search debounce
  "mastdiff.maxResults": 500,                   // cap on displayed results
  "mastdiff.historySize": 50                    // how many searches to keep
}
```

## Keyboard shortcuts

- `Ctrl+Alt+F` (Windows/Linux) or `Cmd+Alt+F` (macOS): focus the search view
- `Enter` in the search box: fire immediately (useful if debounce is slow)
- `Ctrl+K Ctrl+O` (or menu): find calls to the word under cursor
- `Ctrl+K Ctrl+D` (or menu): find definitions of the word under cursor

(The context-menu shortcuts are also registered if your editor language is `cpp`.)

## Troubleshooting

**"mastdiff binary not found":**
- Ensure `mastdiff` is on your `PATH`: `which mastdiff` (Unix) or `where mastdiff` (Windows)
- Or set `mastdiff.binaryPath` to the full path (e.g. `/usr/local/bin/mastdiff`)

**Extension won't activate:**
- Reload VS Code (`Ctrl+Shift+P` → "Reload Window")
- Check the Output panel: select "mastdiff" from the dropdown to see logs

**No results, or very slow:**
- Run `mastdiff --search "fn:" /your/project` in a terminal to verify the CLI works
- If the query returns results, it's likely a settings issue (include/exclude globs filtering out files)
- Check `mastdiff.include` and `mastdiff.exclude` settings

**Results don't update:**
- Press `Ctrl+Shift+F5` to reload the extension (if in dev mode)
- In production, try reloading VS Code (`Ctrl+Shift+P` → "Reload Window")
