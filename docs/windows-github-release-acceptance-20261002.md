# Windows GitHub 分发验收，2026-10-02

发布路线为未签名 x64 EXE 安装器，另提供便携 ZIP；Microsoft Store 与付费签名暂缓。
构建脚本不创建公开 Release 或 Git 标签。

本机 Windows 11 Pro 24H2，10.0.26100，x64，Intel Iris Xe。
窗口 DPI 以各轮独立 UIA 客户端结果中的 `native_dpi` 实测记录为准。
实际验证使用 Release 构建与 Inno Setup 7.1.0，安装界面含英、简中、繁中、日、韩。
未生成或导入签名证书，未改变信任、Developer Mode 或安全策略。

| 验证 | 结果 |
| --- | --- |
| ZIP 解压及文件审计 | 19 个文件，主程序与助手为 x64 PE32+，主程序为 GUI，版本、图标、PMv2 资源通过 |
| ZIP 构建与损坏拒绝 | 18 项通过，含正式构建拒绝 Debug / 脏源码、校验漂移、路径穿越和重复条目 |
| ZIP 运行 | HWND、独立 UIA 38 项、实际公式 / Mermaid 助手通过 |
| EXE 安装器 | SHA256 与审计一致，确认未签名，实际当前用户安装通过 |
| 安装后运行 | 20 个载荷文件完整，开始菜单目标与卸载注册正确，HWND / UIA 38 项及公式 / Mermaid 通过 |
| 同版本重装 | 程序载荷校验与用户文档保持通过 |
| 实际卸载 | 程序、助手、开始菜单与卸载注册移除，安装目录内和外部用户文档均保留 |
| MSIX 回归 | 既有未签名开发包解包检查与原门禁保留，MSIX 安装不计入此次结果 |

复现：先运行 `build-package.ps1 -Channel GitHub -Smoke -TestPipeline`，再对输出目录运行
`build-installer.ps1 -ReleaseDirectory <directory> -CompilerPath <ISCC.exe> -Smoke`。
正式构建要求干净 Release 源码，本地及 CI 候选须显式 `-Candidate`。

最终本机生成物与审计放在 `artifacts/windows-group7/github-release/`：
`Yu-0.1.0-windows-x64-setup.exe`、`Yu-0.1.0-windows-x64.zip`、`SHA256SUMS.txt`、
`release-manifest.json`、`verification.json`、`installer-manifest.json` 与
`installer-verification.json`。各审计文件记录实际源码提交、候选状态、哈希与验证目录；
损坏拒绝测试日志、独立 UIA 结果以及保留下来的样本文档位于相关测试目录。

本轮不声明 Windows 10、跨机器、普通非管理员账户、跨版本升级、互联网下载后的
SmartScreen / Smart App Control 放行、Store 认证或 MSIX 安装已验收。
当前会话账户是 Administrator；安装实际使用 HKCU 与用户开始菜单，模板要求
`PrivilegesRequired=lowest`，但仍需要普通标准账户的独立验证。
未签名文件可能被 Windows 提示或策略阻止；GitHub 与 SHA256 不替代发布者签名。
详见 [Microsoft SmartScreen 文档](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
与 [打包说明](windows-packaging.md)。
