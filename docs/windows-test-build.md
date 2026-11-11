# Windows x64 自主测试安装包

当前源码为 `1.0.3` 集成候选，包含 BidiAppend `data_binary` 兼容修复及社区 PR 56/57/58。它不是原维护者的官方发行版，也未经过真实 Windows / Cursor 端到端验证。历史 `1.0.2` 测试 exe 不覆盖、不重打包；下文命令用于今后显式发起的构建，不表示本次已执行构建。

## GitHub Actions 手动构建与下载

1. 将审核后的代码推送到个人 Fork [Accompany-cc/cursor-byok](https://github.com/Accompany-cc/cursor-byok)，不要向上游发布。要看到手动运行入口，工作流还需存在于该仓库的默认分支，并启用 Actions。
2. 在 **Actions → Windows test build (manual) → Run workflow** 中选择包含修复的分支。工作流构建所选 ref，不会切换回上游 `main`。
3. 成功后下载 artifact `cursor-byok-windows-x64-test-<完整提交 SHA>`，解压取得 NSIS 安装包。产物保留 14 天；提交 SHA 用于确认构建代码。此流程不创建 Release，也不自动修改版本号。
4. 在 Windows x64 上先备份 BYOK 配置，退出正在运行的 Cursor BYOK，再安装测试包。测试包沿用原应用标识，可能覆盖已有 BYOK 安装并复用配置；它不是 Cursor 客户端安装包。

工作流使用 Node.js 22、Rust stable 和 `windows-latest`。它先在仓库根目录执行 `cargo test --locked --package cursor-server --lib api::cursor`，失败会阻止打包；然后在 `apps/desktop` 执行 `npm ci` 和 `npm run tauri:build -- --target x86_64-pc-windows-msvc --bundles nsis --config windows-test.config.json`。

## Linux 本地交叉构建

Tauri 支持用 Linux 构建 Windows MSVC 程序和 NSIS 安装包，但不等同于通过了 Windows 运行测试。

所需工具：Rust stable、`x86_64-pc-windows-msvc` 标准库、Node.js 22、LLVM/Clang/LLD、NSIS、Ninja、NASM 和 `cargo-xwin`。当前 Microsoft CRT 的 C++ 头文件要求 Clang 19 或更高版本，本机使用 LLVM/Clang/LLD 20；Clang 18 会在 `libmimalloc-sys` 编译阶段触发 STL1000 错误，不能通过关闭版本检查来绕过。`cargo-xwin` 会下载 Microsoft CRT 和 Windows SDK，应遵守对应 SDK 许可。

在 `apps/desktop` 执行：`npm run tauri:build -- --runner cargo-xwin --target x86_64-pc-windows-msvc --config windows-test.config.json --ci -- --locked`。

`bundle.targets` 已在测试配置中设为 `nsis`。Linux 版 CLI 的 `--bundles` 参数可能只列出 Linux 格式，因此交叉构建时通过配置选择 NSIS，不传 `--bundles nsis`。以下路径均相对于仓库根目录：

- Windows x64 主程序：`target/x86_64-pc-windows-msvc/release/cursor-byok-desktop.exe`。
- 安装包：`target/x86_64-pc-windows-msvc/release/bundle/nsis/Cursor BYOK_1.0.3_x64-setup.exe`。

`windows-test.config.json` 随源码提供，Windows workflow 与本地交叉构建共用；主配置与测试配置均不生成 updater 产物，均无更新地址或签名公钥。

## 自动更新与签名

- 测试配置设置 `bundle.createUpdaterArtifacts: false`，不生成 updater 签名产物，无需上游 `TAURI_SIGNING_PRIVATE_KEY` 等 secrets。
- 测试配置将 `plugins.updater.endpoints` 设为空数组、`plugins.updater.pubkey` 设为空字符串。Tauri v2 updater 配置没有 `active` 开关。
- 当前源码在前端检查/安装入口和 Rust Windows 检查/安装命令中直接禁用更新，同时不注册 Tauri updater 插件。不会请求更新元数据、下载更新包或提示“已是最新版本”；设置页明确要求手动下载。保留的 Windows endpoint 仅指向本 Fork，但禁用期间不会访问。
- 历史 `1.0.2` 测试 exe 的行为不同：Windows 硬编码地址仍可能请求上游元数据，清空公钥只能阻止安装。此限制不会因编辑源码而修复旧 exe；不要把历史包当作已完全禁用更新的新包。
- Tauri updater 签名与 Windows Authenticode 代码签名不同。本测试包不配置这两类签名，Windows 可能提示未知发布者或 SmartScreen 警告。核对仓库、文件校验值和构建记录，不要全局关闭系统安全功能。
- 若系统缺少 Microsoft Edge WebView2 Runtime，默认 NSIS 安装流程会联网下载安装。

## 验证边界

Linux 构建、归档完整性和 PE 格式检查不能替代真实 Windows 测试。请使用未打二进制编码补丁的 Cursor 验证安装启动、普通对话、工具调用、子代理和多轮续聊，并保留正常的代理及证书配置。本流程不自动提交或推送代码，也不修改 Cursor 客户端安装文件。
