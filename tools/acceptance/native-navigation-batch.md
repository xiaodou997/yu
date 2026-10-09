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

Print keeps its existing Command+P shortcut. Folder selection does not modify or close the current document. macOS folder access uses existing sandbox grants. Folder roots persist through in-window document switches; this batch does not add a saved project format, persistent full-text index, or automatic document reopening. A selected workspace folder is now remembered locally across launches; unavailable folders fall back without replacing the current document.

## Combined scenarios

1. Open a folder containing nested Markdown, .markdown, .mdown and .txt files, duplicate basenames, spaces, Chinese, emoji and long paths. Browse folders and switch files without changing the chosen root. On Windows, enter a directory and use `..` to return, stopping at the chosen root.
2. Quick open: search by basename, partial name, relative path and spaced path tokens. Use Up/Down, Return, Escape, Tab/Shift+Tab, double-click and Open. Check zero results, native IME composition, cancellation during scanning, and focus return. Confirm print and ordinary Open shortcuts remain unchanged.
3. Change a file outside the app: create, remove and rename. Refresh the sidebar; reopen or refresh quick open. A stale/deleted selection must not open a different file. Large/limited scans must say the index is partial, not claim exhaustive results.
4. Keep unsaved edits, use quick open, cancel the existing Save/Discard prompt, and verify exact source, selection and undo remain intact. Repeat with Save and Discard. Folder-only navigation must not prompt to close the document. Check external-save conflicts and already-open-file deduplication on macOS.
5. Find/replace: literal search with Match Case and Whole Words options; test Straße/STRASSE, Greek sigma, dotted I, combining marks, underscore and CJK boundaries; check option changes refresh highlights without editing source; single/all/empty replacements; adjacent deletions; replacement containing the query; no-op replacement; single-step undo/redo; source/visual mode and long-file navigation. Do not let IME Return/Escape accidentally execute commands.
6. Run the entire batch in light/dark/high-contrast appearance where supported, five UI languages, and Windows 100/125/150/200% scaling. Check long-label layout, accessible field names, keyboard focus visibility, multi-monitor DPI changes, and disabled controls.

## Automation included

- Shared metadata scan/ranking/budget/cancellation/symlink/stale-path tests in `yu-storage`.
- Folder navigation preserves dirty source/revision and workspace identity in the Windows model.
- macOS search-panel self-check also exercises the workspace C ABI and Unicode paths.
- macOS launch-window self-check exercises the native quick-open sheet, query focus, selection and teardown.
- Windows source and tests can be type-checked separately on macOS, but this is not Windows execution or installer validation.

## Intentional first-batch limits

Quick open indexes names/paths only, not file contents. It skips hidden entries, symlinks/reparse targets and known build/dependency directories. The index is capped at 5,000 files, 50,000 examined entries, 32 directory levels and a cooperative two-second scan budget; individual filesystem calls can take longer. Results display at most 100 files. Reopening/Refresh scans again. Workspace setup changes application state only and writes no private files inside the selected directory.

Search remains literal within the current document, case-sensitive by default. Optional caseless matching uses full Unicode default folding and preserves complete source-scalar boundaries (never half of ß); Whole Words uses UAX #29 boundaries, not dictionary-based CJK segmentation or accent/width normalization. Regex, selection-only replace, multi-root workspaces and cross-file replacement remain outside this batch.

## Navigation history and restoration batch

- macOS retains its native recent-document menu and adds bounded recent folders in UserDefaults, reusing existing sandbox bookmarks. Windows adds bounded recent files/folders stored as one native registry value under HKCU\\Software\\YuMarkdown\\Navigation. Neither stores document contents or writes into the selected workspace.
- Open two workspaces, relaunch, and check the last folder restores. Open a recent folder while a document is dirty: source, selection and undo must stay unchanged. Save As inside a pinned workspace must not reset its root.
- Recent-file activation must reuse Save/Discard/Cancel and same-file detection; cancel must not record a file that was never opened. Deleted paths must be disabled or fail safely. Clear Recent Files and Clear Recent Folders independently; clearing must neither delete files nor close the active document/workspace. Clearing folders also clears automatic next-launch restoration.
- Test permissions/bookmark expiry, malformed/unsupported-version registry data, non-ASCII paths, independent app instances and process relaunches in the final test-machine run. Automated checks use isolated preferences or a pure codec, not the user's real history.
