# ADR-0086：用 `yu-storage::DocumentSession` 固定文件持久化边界

## 状态

已接受（2026-08-13）

## 背景

`EditorDocument` 已经是 canonical Markdown source、Revision、selection、history 和 composition
的状态边界，但打开/保存文件如果直接散落在平台窗口中，会重新产生“磁盘 source、TextKit mirror、
编辑器 source”三套真源。BOM、外部文件替换、保存失败和 undo 后 dirty 语义也会被各个平台分别解释。

## 决策

新增独立 `yu-storage` crate，提供 headless `DocumentSession`：

- `open` 只接受 UTF-8；UTF-8 BOM 作为 `Utf8Bom` 元数据保留，不进入 Markdown source 坐标；
- `new` 创建不存在路径的未保存 session；目标文件若在首次保存前出现，保存仍会拒绝覆盖；
- 永久编辑通过 `EditorDocument::execute`/`apply_transaction`，IME preedit 通过同一 document 的
  transient composition API；storage 不复制 source；
- dirty 由当前 Revision 与 `saved_revision` 比较。即使 undo 回到同样字节，新的 Revision 仍需显式
  保存，避免把“内容相同”误当成“已经确认写盘”；
- 保存先比较打开/上次保存时的文件指纹（长度、修改时间、FNV-1a 内容摘要），再在同目录创建唯一
  临时文件、写入/`sync_all`、保留原权限并 rename 替换；外部改变或删除时返回冲突，不覆盖目标；
- reload 只允许 clean session；读取新文件后通过 `EditorDocument::reset_source` 重新建立 parser、
  selection、cache 和 Revision，而不是在 storage 层维护第二份文本。

## 结果

macOS、未来其他平台和 CLI 可以共享相同的 open/save/conflict/BOM 行为；平台 UI 只需展示
`StorageError`、触发保存/重载，不需要自行判断 dirty 或复制 Markdown。headless 测试覆盖 BOM、原子保存、
外部替换/删除、dirty reload、composition commit/cancel 和 invalid UTF-8。

## 限制

当前只支持有效 UTF-8 Markdown，不做编码猜测、换行规范化、文件监听、autosave、目录 workspace、
软链接策略或文件权限/owner 的跨平台扩展；这些在接入 macOS 文档窗口前单独定义。保存使用同目录
rename，但不承诺跨文件系统移动的原子性。
