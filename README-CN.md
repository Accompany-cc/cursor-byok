<div align="center">

# cursor-byok

在 Cursor 中使用你自己的模型 API。

[下载](https://github.com/Accompany-cc/cursor-byok/releases) · [提交问题](https://github.com/Accompany-cc/cursor-byok/issues) · [English](./README.md)

[![Release](https://img.shields.io/github/v/release/Accompany-cc/cursor-byok?style=flat-square)](https://github.com/Accompany-cc/cursor-byok/releases)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](./LICENSE)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey?style=flat-square)

</div>

![将 cursor-byok 连接到多种模型 API](./images/en-brand-1.png)

![cursor-byok 控制面板](./images/en-home-1.png)

## 项目简介

cursor-byok 是一个开源的本地 Cursor 模型网关。连接你自己的 OpenAI、Anthropic 兼容 API，继续使用工具调用、Skills、MCP 和多轮对话等 Agent 能力。

## 个人维护说明 / Personal fork

这是 [Accompany-cc](https://github.com/Accompany-cc) 以个人自用为主的维护分支，不是原作者或社区维护者的官方发行版。欢迎 Issue 和 PR，但功能取舍、支持范围和合并节奏优先服务自用需求，不承诺响应时间。

代码源自 [leookun/cursor-byok](https://github.com/leookun/cursor-byok)，集成基线来自 [leehainuo/cursor-byok](https://github.com/leehainuo/cursor-byok) 的 `e69bda0f6a8da4f002b934bc86c572d32d824800`，并整合其 [PR 56](https://github.com/leehainuo/cursor-byok/pull/56)、[PR 57](https://github.com/leehainuo/cursor-byok/pull/57)、[PR 58](https://github.com/leehainuo/cursor-byok/pull/58) 及本地 BidiAppend 二进制兼容修复。感谢原作者、社区维护者和 PR 贡献者；这些来源不表示其对本分支的背书。

当前源码版本为 **1.0.3 候选代码**，尚未经过真实 Windows / Cursor 端到端验证，不代表已发布或已验证的安装包。原 `1.0.2` 测试 exe 保留不变，不由本次源码升级覆盖或重打包。验证边界见 [兼容修复说明](./docs/bidi-binary-compat.md) 和 [Windows 测试说明](./docs/windows-test-build.md)。

此分支没有配置独立 updater 签名：**自动更新检查、下载和安装均禁用**，请仅从本仓库 Releases 手动下载，不会自动回退到上游。仍沿用 `Cursor BYOK` 产品名、`dev.cursorbyok.desktop` 应用标识及原配置目录，以便迁移；安装可能覆盖旧版并复用原配置，升级前请退出应用并备份配置（包含 API Key）。

## 主要功能

- **自定义模型**：配置 API 地址、密钥、模型 ID 和请求参数。
- **模型管理**：添加、编辑、排序模型配置，并批量测试连接。
- **连接性能测试**：测量响应延迟和生成速度。
- **会话指标**：查看 Token 用量、缓存命中和对话活动。
- **跨平台运行**：支持 macOS、Windows 和 Linux。

![cursor-byok 模型设置](./images/en-model-1.png)

## 工作原理

```text
Cursor 客户端
    │
    │ Agent 请求与工具结果
    ▼
cursor-byok 本地服务
    │
    │ OpenAI / Anthropic 兼容请求
    ▼
你配置的模型 API
```

cursor-byok 在本机处理协议适配、模型请求转发、工具调用协调和会话状态。API Key 与应用设置保存在本地；模型请求仍会发送到你配置的服务商。

## 项目初衷

许多 Agent 产品将工具能力与固定的模型、订阅和计费方案绑定，用户只能使用平台提供的模型通道。

cursor-byok 希望将模型选择权交还给用户。开发者可以充分利用已有的 API 和额度，选择适合自己的模型与服务商，并在需要时自行部署相关服务。

## 致谢

本个人维护分支保留 leookun 原始项目、leehainuo 社区基线以及以上 PR 的来源与致谢；由 [Accompany-cc](https://github.com/Accompany-cc) 维护当前分支。

项目采用 [MIT License](./LICENSE)。原作者版权声明和完整许可文本见[第三方许可](./THIRD-PARTY-LICENSES.md)。
