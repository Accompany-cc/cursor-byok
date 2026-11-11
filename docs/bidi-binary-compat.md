# BidiAppend 二进制载荷兼容修复

## 开发基线与范围

- 基础仓库：`https://github.com/leehainuo/cursor-byok`。
- 基础提交：`e69bda0f6a8da4f002b934bc86c572d32d824800`。
- 本地开发分支：`fix/accept-bidi-append-data-binary`。
- 目标：让代理直接接受 Cursor 发出的二进制 BidiAppend 消息，不再为这次编码变化修改 Cursor 安装文件。
- 历史 `1.0.2` 测试构建使用独立覆盖配置；原 exe 保留不变。本次集成源码为 `1.0.3` 候选，含社区 PR [56](https://github.com/leehainuo/cursor-byok/pull/56)、[57](https://github.com/leehainuo/cursor-byok/pull/57)、[58](https://github.com/leehainuo/cursor-byok/pull/58)。当前 Fork 的生产配置和代码均禁用更新检查与安装，不再使用上游签名或更新通道。

## 故障及处理规则

`BidiAppendRequest` 已定义 `bytes data_binary = 4`，但 `server/src/api/cursor/bidi.rs` 的 Agent 解码入口原先直接拒绝非空 `data_binary`。因此，使用二进制编码的请求在进入模型执行前失败。

新的载荷选择规则：

1. 只有 `data_binary` 非空时，按原始 protobuf 解码。
2. 只有 `data` 非空时，先按十六进制解码，再解码 protobuf。
3. 两者都为空时返回协议错误。
4. 两者同时非空时拒绝请求（即使内容相同），不采用二进制优先或静默回退；此规则与集成的 PR #57 一致。
5. 保留请求 ID 校验、protobuf 解码、模型路由和消息序号处理。

这项改动只覆盖消息载荷编码兼容，不保证未来所有 Cursor 协议变化都无需适配，也不等同于已经通过真实 Windows Cursor 的端到端验证。

## 测试

`server/src/api/cursor/bidi/tests.rs` 使用实际非空 `AgentRunRequest`，校验新旧编码、请求 ID、序号、双字段歧义拒绝、空载荷、非法十六进制与损坏的 protobuf。

HTTP 路由回归位于 `server/src/api/cursor/handlers/run_inference_tests.rs`：

- 以两种编码分别启动普通 Agent，通过 HTTP 接收输出并确认假模型提供方实际收到请求。
- 以两种编码分别运行客户端 `RunInference` 流程，校验文本、工具调用、完成事件。
- 异常载荷返回 HTTP 400，且不调用模型。

测试使用本地临时数据库和假模型提供方，不需要真实 API 密钥，也不需要访问模型服务。

原修复说明中的测试示例不能直接照搬：`AgentClientMessage::default()` 编码后为空，不能作为成功载荷；`ai::BidiAppendRequest` 的请求 ID 字段应构造为 `ai::BidiRequestId`，不能使用 `agent::BidiRequestId`。

## 本地验证命令

在仓库根目录执行，需要 Rust stable；服务端构建需要 C 编译器、CMake、pkg-config 和 OpenSSL 开发库。

- `cargo fmt --all -- --check`
- `cargo test --locked --package cursor-server --lib api::cursor`
- `cargo test --locked --package cursor-server`
- `cargo clippy --locked --package cursor-server --all-targets -- -D warnings`
- `cargo build --locked --release --package cursor-server --bin cursor-server`
- `node .github/scripts/check-release.mjs`
- `node --test .github/scripts/*.test.mjs`

前端使用 Node.js 22 或更高版本，执行 `npm --prefix apps/desktop ci` 和 `npm --prefix apps/desktop run check`。本机系统仍为 Node.js 18，可在不替换系统版本的情况下复跑：`npm exec --yes --package=node@22 --package=npm@10 -- sh -c 'npm --prefix apps/desktop ci && npm --prefix apps/desktop run check'`。

完整工作区检查仍可使用上游的 `make check`，但需要 Linux 桌面端的 GTK/WebKit 等开发依赖。

## Windows 验证

Linux 服务端产物不能直接在 Windows 上运行。Windows 测试包流程见 [Windows 测试构建说明](windows-test-build.md)。

在未打客户端补丁的 Cursor 上，至少验证普通对话、工具调用、子代理和多轮续聊。保留正常代理及证书配置；本修复替代的是二进制编码绕过补丁，不是代理自身所需的连接设置。

## 验证记录

以下是 2026-11-10 对当时 `1.0.2` 代码的历史记录，不代表 `1.0.3` 集成候选或真实 Windows 已验证。

验证日期：2026-11-10；环境：Linux x86_64、Rust 1.99.1、Node.js 22.23.4。

- 修复前：新增测试与原有相关测试共运行 19 项，15 项通过、4 项失败。失败为 3 项二进制解码成功场景和 1 项二进制 Agent HTTP 请求场景；旧编码和二进制 RunInference 流程正常，准确复现原故障。
- 格式检查：通过。
- 发布配置校验及 6 项发布脚本测试：通过。
- 前端类型检查、Node 配置类型检查与生产构建：通过；构建仍有上游现存的大包和动态导入提示。
- 修复后的服务端全量测试：293 项全部通过，其中包含新旧编码、HTTP 模型调用路径、RunInference、子代理多轮历史等回归。
- Clippy：`cursor-server --all-targets` 在 `-D warnings` 下通过。
- Linux release 构建：成功，产物为 `target/release/cursor-server`。这是 Linux x86_64 服务端程序，不能作为 Windows 安装包使用。
- Windows workflow 静态配置已检查，Windows 实际构建与真实 Cursor 验证尚未执行。
- 本机未执行 Linux 桌面端打包或整个 Rust workspace 的检查；当前 Rust 验证范围为 `cursor-server` 包。

本机安装了 Rust 和缺少的原生构建依赖。系统原有 Node.js 18 未替换；前端检查通过临时 Node.js 22 运行。依赖下载使用了命令行临时指定的镜像，未将镜像配置或依赖版本变更写入仓库。
