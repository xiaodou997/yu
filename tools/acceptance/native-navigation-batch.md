# Native navigation batch acceptance

Development batch: find/replace, quick open, and lightweight folder workspace.
These features share the canonical Markdown editor and existing safe document-switch path. Native Windows execution and packaged-app acceptance remain pending until the combined test-machine run.

## Entry points

| Action | macOS | Windows |
| --- | --- | --- |
| Find | Command+F | Ctrl+F |
| Find and replace | Option+Command+F | Ctrl+H |
| Quick open | Shift+Command+O | Ctrl+Shift+O |
| Open folder | Option+Command+O | Ctrl+Alt+O |
| Refresh files / use document folder | File menu | File menu |

Print keeps its existing Command+P shortcut. Folder selection does not modify or close the current document. macOS folder access uses existing sandbox grants. Folder roots persist through in-window document switches; this batch does not add a saved project format, persistent full-text index, or automatic cross-process workspace restoration.

## Combined scenarios

1. Open a folder containing nested Markdown, .markdown, .mdown and .txt files, duplicate basenames, spaces, Chinese, emoji and long paths. Browse folders and switch files without changing the chosen root. On Windows, enter a directory and use `..` to return, stopping at the chosen root.
2. Quick open: search by basename, partial name, relative path and spaced path tokens. Use Up/Down, Return, Escape, Tab/Shift+Tab, double-click and Open. Check zero results, native IME composition, cancellation during scanning, and focus return. Confirm print and ordinary Open shortcuts remain unchanged.
3. Change a file outside the app: create, remove and rename. Refresh the sidebar; reopen or refresh quick open. A stale/deleted selection must not open a different file. Large/limited scans must say the index is partial, not claim exhaustive results.
4. Keep unsaved edits, use quick open, cancel the existing Save/Discard prompt, and verify exact source, selection and undo remain intact. Repeat with Save and Discard. Folder-only navigation must not prompt to close the document. Check external-save conflicts and already-open-file deduplication on macOS.
5. Find/replace: literal case-sensitive search; single/all/empty replacements; adjacent deletions; replacement containing the query; no-op replacement; single-step undo/redo; source/visual mode and long-file navigation. Do not let IME Return/Escape accidentally execute commands.
6. Run the entire batch in light/dark/high-contrast appearance where supported, five UI languages, and Windows 100/125/150/200% scaling. Check long-label layout, accessible field names, keyboard focus visibility, multi-monitor DPI changes, and disabled controls.

## Automation included

- Shared metadata scan/ranking/budget/cancellation/symlink/stale-path tests in `yu-storage`.
- Folder navigation preserves dirty source/revision and workspace identity in the Windows model.
- macOS search-panel self-check also exercises the workspace C ABI and Unicode paths.
- macOS launch-window self-check exercises the native quick-open sheet, query focus, selection and teardown.
- Windows source and tests can be type-checked separately on macOS, but this is not Windows execution or installer validation.

## Intentional first-batch limits

Quick open indexes names/paths only, not file contents. It skips hidden entries, symlinks/reparse targets and known build/dependency directories. The index is capped at 5,000 files, 50,000 examined entries, 32 directory levels and a cooperative two-second scan budget; individual filesystem calls can take longer. Results display at most 100 files. Reopening/Refresh scans again. Workspace setup changes application state only and writes no private files inside the selected directory.

Search remains literal and case-sensitive in the current document. Regex, selection-only replace, persistent recent-workspace lists, multi-root workspaces and cross-file replacement are not included in this batch.
