# ADR 0106：macOS 受控 HTML 粘贴 native adapter

## 状态

已接受（Phase 2，macOS host）。Rust HTML policy 与 C ABI 已接入 `DocumentTextView`；跨平台
adapter、真实跨应用回归和粘贴失败遥测仍待后续。

## 背景

`yu-export::import_html_fragment` 已经把 HTML→Markdown 限制在一个可拒绝的语义子集。macOS
`DocumentTextView` 仍需要处理 `NSPasteboard` 的多种类型，并把最终字符串交回同一个
`DocumentEditorSession`，不能让 Swift 维护富文本副本或绕过 Revision-bound `insert_text`。

## 决策

- macOS pasteboard 的消费顺序固定为：`net.daringfireball.markdown` → `public.utf8-plain-text`
  (`NSPasteboard.string`) → `public.html` 的受控 HTML→Markdown 导入。
- Markdown UTI 和纯文本都是已有 source，直接交给当前 session；只有前两者都不存在时才调用
  无状态 `yu_storage_import_html_fragment`。该 C ABI 使用两次查询返回 owned UTF-8，不持有
  pasteboard、session 或 Rust buffer 指针。
- HTML policy 拒绝统一映射为 `YU_STORAGE_HTML_IMPORT_REJECTED`。Swift 将其视为“没有可安全粘贴的
  HTML”，不执行、不显示、不注入原始 HTML；若未来 adapter 改为 HTML 优先，仍必须把拒绝转为
  `text/plain`，不能绕过策略。
- 成功导入的 Markdown 仍通过 `StorageBridge.insertText` 进入当前 Revision、selection、dirty、
  undo/history 和 source mirror 链路；HTML importer 本身不改变文档。
- `hasSourceOnPasteboard` 只检查类型存在，不在菜单验证期间解析 HTML；解析只发生在用户真正执行
  paste 时，避免主线程菜单查询重复做工作。
- `--clipboard-self-check` 使用私有 pasteboard 验证 Markdown、纯文本、HTML fallback 和拒绝路径，
  并读取 `experiments/macos-document-host/Fixtures/clipboard` 的 semantic mail、GFM table、
  browser wrapper、unsafe HTML fixture；它不启动窗口、不改写系统剪贴板。

## 验证

```bash
cargo test -p yu-storage-ffi -p yu-export
experiments/macos-document-host/build-app.sh
experiments/macos-document-host/.build/YuMacDocumentHost.app/Contents/MacOS/YuMacDocumentHost \
  --clipboard-self-check experiments/macos-document-host/Fixtures/sample.md
```

fixture self-check 已覆盖四类来源：只提供受控 semantic HTML 的邮件样式片段、带对齐的 table、带
`html/body/div`/fragment marker 的浏览器包装片段，以及脚本片段。后两类按策略拒绝；如果 pasteboard
同时带纯文本，native adapter 使用纯文本，不把拒绝原因暴露为崩溃或 HTML 注入。

## 后续

在 macOS 真实跨应用粘贴中覆盖浏览器、邮件和只提供 HTML 的来源，记录 fixture 之外的策略拒绝与
纯文本回退；随后
把相同格式契约映射到 Windows TSF/clipboard 和 Linux Wayland/X11 clipboard adapter。
